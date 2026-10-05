//! `/model auto`: choose a registry model for the detected hardware.
//!
//! The rule is "best measured (or provisionally ranked) model that runs
//! comfortably", not "largest model". A model is considered if:
//!   * it is eligible for automatic selection,
//!   * its license is in the permitted set,
//!   * it supports tool calling,
//!   * weights + KV cache for at least `min_context` fit in fast memory, or a
//!     MoE model fits with expert layers offloaded to system RAM.
//!
//! The context length is reduced (down to `min_context`) before rejecting a
//! model. Disk space is checked separately and reported.

use crate::local::hardware::{format_bytes, HardwareInfo};
use crate::local::registry::{ModelSpec, Registry};
use serde::Serialize;

pub const PERMITTED_LICENSES: &[&str] = &["Apache-2.0", "MIT", "BSD-3-Clause"];

#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum Placement {
    /// Everything in GPU / unified memory (or RAM for CPU-only machines).
    Full,
    /// MoE experts offloaded to system RAM; slower but workable.
    PartialOffload,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Candidate {
    pub id: String,
    pub name: String,
    pub fits: bool,
    pub context: u32,
    pub memory_needed: u64,
    pub placement: Option<Placement>,
    pub disk_ok: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Recommendation {
    pub model: Option<ModelSpec>,
    pub context: u32,
    pub placement: Option<Placement>,
    pub budget: u64,
    pub candidates: Vec<Candidate>,
    pub summary: String,
}

fn plan_for(m: &ModelSpec, hw: &HardwareInfo) -> (Option<(u32, Placement)>, String) {
    let fast = hw.fast_memory_budget();
    let mut ctx = m.default_context;
    loop {
        let need = m.memory_needed(ctx);
        if need <= fast {
            return (
                Some((ctx, Placement::Full)),
                format!(
                    "fits in {} with {ctx} tokens of context",
                    format_bytes(fast)
                ),
            );
        }
        if ctx <= m.min_context {
            break;
        }
        ctx = (ctx / 2).max(m.min_context);
    }
    // MoE models run acceptably with experts in system RAM when a discrete
    // GPU holds attention and the KV cache.
    if m.is_moe() && hw.has_discrete_gpu() {
        let ctx = m.min_context.max(m.default_context / 2);
        let kv = m.kv_bytes_per_token * ctx as u64;
        let ram = hw.cpu_memory_budget();
        if kv + 2_000_000_000 <= fast && m.size_bytes <= ram + fast - kv {
            return (
                Some((ctx, Placement::PartialOffload)),
                format!(
                    "MoE: experts offloaded to RAM ({} RAM + {} VRAM)",
                    format_bytes(ram),
                    format_bytes(fast)
                ),
            );
        }
    }
    (
        None,
        format!(
            "needs about {} at {} tokens; {} available",
            format_bytes(m.memory_needed(m.min_context)),
            m.min_context,
            format_bytes(fast)
        ),
    )
}

pub fn recommend(registry: &Registry, hw: &HardwareInfo) -> Recommendation {
    let budget = hw.fast_memory_budget();
    let mut candidates = Vec::new();
    let mut eligible: Vec<(&ModelSpec, u32, Placement, bool)> = Vec::new();

    for m in &registry.models {
        let disk_ok = hw
            .disk_free
            .map(|f| f > m.size_bytes + 1_000_000_000)
            .unwrap_or(true);
        let mut c = Candidate {
            id: m.id.clone(),
            name: m.name.clone(),
            fits: false,
            context: 0,
            memory_needed: m.memory_needed(m.min_context),
            placement: None,
            disk_ok,
            reason: String::new(),
        };
        if !PERMITTED_LICENSES.contains(&m.license.as_str()) {
            c.reason = format!("license {} not in the permitted set", m.license);
        } else if m.tool_calling == "none" {
            c.reason = "no tool calling support".into();
        } else {
            let (plan, why) = plan_for(m, hw);
            c.reason = why;
            if let Some((ctx, placement)) = plan {
                c.fits = true;
                c.context = ctx;
                c.memory_needed = m.memory_needed(ctx);
                c.placement = Some(placement.clone());
                if !m.auto_select {
                    c.reason
                        .push_str("; not auto-selected (pending evaluation or too small)");
                } else {
                    eligible.push((m, ctx, placement, disk_ok));
                }
            }
        }
        candidates.push(c);
    }

    // Best score first; full placement beats partial offload at equal score.
    // Measured scores are only comparable with each other: use them when every
    // candidate has one, otherwise fall back to the provisional rank for all.
    let all_measured = eligible.iter().all(|e| e.0.eval_score.is_some());
    let score = |m: &ModelSpec| {
        if all_measured {
            m.eval_score.unwrap_or(0.0) * 100.0
        } else {
            m.quality_rank as f64
        }
    };
    eligible.sort_by(|a, b| {
        let sa = score(a.0)
            - if a.2 == Placement::PartialOffload {
                15.0
            } else {
                0.0
            };
        let sb = score(b.0)
            - if b.2 == Placement::PartialOffload {
                15.0
            } else {
                0.0
            };
        sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
    });

    match eligible.first() {
        Some((m, ctx, placement, disk_ok)) => {
            let mut summary = format!(
                "{} ({}, {:.1} GB download) with {} tokens of context, {}",
                m.name,
                m.quantization,
                m.size_gb(),
                ctx,
                match placement {
                    Placement::Full => format!("fully in {}", if hw.unified_memory { "unified memory" } else if hw.has_discrete_gpu() { "GPU memory" } else { "RAM" }),
                    Placement::PartialOffload => "experts offloaded to system RAM".to_string(),
                }
            );
            if m.eval_score.is_none() {
                summary.push_str(". Ranking is provisional until Kara evaluation results exist");
            }
            if !disk_ok {
                summary.push_str(". WARNING: not enough free disk space for the download");
            }
            Recommendation {
                model: Some((*m).clone()),
                context: *ctx,
                placement: Some(placement.clone()),
                budget,
                candidates,
                summary,
            }
        }
        None => Recommendation {
            model: None,
            context: 0,
            placement: None,
            budget,
            candidates,
            summary: format!(
                "No registry model fits in about {} of usable memory. You can point Kara at another local runtime (Ollama, LM Studio) with a smaller model.",
                format_bytes(budget)
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::hardware::Gpu;

    fn mac(gb: u64) -> HardwareInfo {
        HardwareInfo {
            os: "macos".into(),
            arch: "aarch64".into(),
            unified_memory: true,
            metal: true,
            total_ram: gb << 30,
            available_ram: (gb / 2) << 30,
            disk_free: Some(500 << 30),
            ..Default::default()
        }
    }

    #[test]
    fn big_mac_gets_the_high_tier() {
        let r = recommend(&Registry::builtin(), &mac(64));
        assert_eq!(r.model.unwrap().id, "qwen3.6-35b-a3b-q4_k_m");
        assert_eq!(r.placement, Some(Placement::Full));
    }

    #[test]
    fn sixteen_gb_mac_gets_a_small_model_not_the_largest() {
        let r = recommend(&Registry::builtin(), &mac(16));
        let m = r.model.unwrap();
        assert!(m.size_bytes < 10_000_000_000, "{}", m.id);
        assert!(r
            .candidates
            .iter()
            .any(|c| c.id == "qwen3.6-35b-a3b-q4_k_m" && !c.fits));
    }

    #[test]
    fn moe_partial_offload_on_mid_gpu() {
        let hw = HardwareInfo {
            os: "linux".into(),
            total_ram: 64 << 30,
            available_ram: 48 << 30,
            cuda: true,
            gpus: vec![Gpu {
                vendor: "NVIDIA".into(),
                name: "RTX 4070".into(),
                vram_bytes: Some(12 << 30),
                ..Default::default()
            }],
            disk_free: Some(200 << 30),
            ..Default::default()
        };
        let r = recommend(&Registry::builtin(), &hw);
        let c = r
            .candidates
            .iter()
            .find(|c| c.id == "qwen3.6-35b-a3b-q4_k_m")
            .unwrap();
        assert_eq!(c.placement, Some(Placement::PartialOffload));
        // A dense 27B does not fit in 12 GB and cannot be partially offloaded usefully.
        assert!(
            !r.candidates
                .iter()
                .find(|c| c.id == "qwen3.6-27b-q4_k_m")
                .unwrap()
                .fits
        );
    }

    #[test]
    fn measured_eval_scores_override_rank() {
        let mut reg = Registry::builtin();
        for m in reg.models.iter_mut() {
            m.eval_score = Some(match m.id.as_str() {
                "qwen3.6-27b-q4_k_m" => 0.7,
                "qwen3.6-35b-a3b-q4_k_m" => 0.6,
                _ => 0.3,
            });
        }
        let r = recommend(&reg, &mac(64));
        assert_eq!(r.model.unwrap().id, "qwen3.6-27b-q4_k_m");
    }

    #[test]
    fn a_single_measured_model_does_not_outrank_unmeasured_ones() {
        let mut reg = Registry::builtin();
        for m in reg.models.iter_mut() {
            if m.id == "qwen3-4b-q4_k_m" {
                m.eval_score = Some(0.5);
            }
        }
        assert_eq!(
            recommend(&reg, &mac(64)).model.unwrap().id,
            "qwen3.6-35b-a3b-q4_k_m"
        );
    }

    #[test]
    fn tiny_machine_gets_nothing_honestly() {
        let r = recommend(
            &Registry::builtin(),
            &HardwareInfo {
                total_ram: 4 << 30,
                available_ram: 2 << 30,
                ..Default::default()
            },
        );
        assert!(r.model.is_none());
        assert!(r.summary.contains("No registry model fits"));
    }

    #[test]
    fn unpermitted_license_is_skipped() {
        let mut reg = Registry::builtin();
        for m in reg.models.iter_mut() {
            m.license = "Proprietary".into();
        }
        assert!(recommend(&reg, &mac(64)).model.is_none());
    }
}
