//! `veyra doctor` and `veyra privacy`.

use crate::app::{display_path, App, Options};
use console::style;
use veyra_context::CommandCategory;
use veyra_core::privacy::PrivacyReport;
use veyra_core::Config;
use veyra_model::hardware::{format_bytes, HardwareInfo};
use veyra_model::recommend::recommend;
use veyra_runtime::llamacpp::{version, LlamaCppManager};
use veyra_runtime::store::ModelStore;

fn row(label: &str, value: impl std::fmt::Display) {
    println!("  {:<20}{}", label, value);
}

fn yes_no(b: bool) -> String {
    if b {
        style("yes").green().to_string()
    } else {
        style("no").dim().to_string()
    }
}

pub fn print_doctor(app: &App, _rt: &tokio::runtime::Runtime) {
    let hw = HardwareInfo::detect(&app.paths.models_dir());
    println!("{}", style("System").bold());
    row(
        "OS",
        format!(
            "{} ({})",
            if hw.os_version.is_empty() {
                &hw.os
            } else {
                &hw.os_version
            },
            hw.arch
        ),
    );
    row("CPU", format!("{} ({} threads)", hw.cpu, hw.cpu_cores));
    row(
        "RAM",
        format!(
            "{} total, {} available",
            format_bytes(hw.total_ram),
            format_bytes(hw.available_ram)
        ),
    );
    if hw.gpus.is_empty() {
        row("GPU", "none detected");
    }
    for g in &hw.gpus {
        let vram = g
            .vram_bytes
            .map(|v| format!(", {} VRAM", format_bytes(v)))
            .unwrap_or_default();
        let name = if g.vendor.is_empty() || g.name.starts_with(&g.vendor) {
            g.name.clone()
        } else {
            format!("{} {}", g.vendor, g.name)
        };
        row("GPU", format!("{name}{vram}"));
    }
    row("Unified memory", yes_no(hw.unified_memory));
    row("Metal", yes_no(hw.metal));
    row("CUDA", yes_no(hw.cuda));
    row("Vulkan", yes_no(hw.vulkan));
    row("ROCm/HIP", yes_no(hw.rocm));
    row(
        "Model memory budget",
        format!(
            "about {} ({})",
            format_bytes(hw.fast_memory_budget()),
            hw.accelerator()
        ),
    );
    row(
        "Disk free",
        hw.disk_free
            .map(format_bytes)
            .unwrap_or_else(|| "unknown".into())
            + &format!(" at {}", display_path(&app.paths.models_dir())),
    );

    println!("\n{}", style("Runtime").bold());
    let mgr = LlamaCppManager::new(&app.paths.runtimes_dir());
    match mgr.locate(&app.config.runtime.llama_server_path) {
        Some(found) => {
            row("llama.cpp", found.describe());
            if let Some(v) = version(found.binary()) {
                row("version", v);
            }
        }
        None => row(
            "llama.cpp",
            format!(
                "not installed (Veyra will offer the pinned build {} on first use)",
                mgr.pin.tag
            ),
        ),
    }
    if let Some(asset) = mgr.pin.select(&hw) {
        row("build for this host", &asset.name);
    }
    row("bind address", &app.config.runtime.bind_host);
    row("provider", app.config.model.provider.label());

    println!("\n{}", style("Models").bold());
    let store = ModelStore::new(&app.paths.models_dir());
    let installed = store.installed();
    if installed.is_empty() {
        row("installed", "none");
    }
    for m in installed {
        row(
            "installed",
            format!(
                "{} ({}, {}, rev {})",
                m.name,
                format_bytes(m.size_bytes),
                m.license,
                &m.revision[..12.min(m.revision.len())]
            ),
        );
    }
    let rec = recommend(&app.models, &hw);
    row("recommendation", &rec.summary);
    for c in rec.candidates.iter().filter(|c| !c.fits).take(3) {
        row("", style(format!("{}: {}", c.name, c.reason)).dim());
    }

    println!("\n{}", style("Repository").bold());
    row("root", app.root.display());
    row("git", yes_no(veyra_context::git::is_repo(&app.root)));
    let langs: Vec<String> = app
        .profile
        .languages
        .iter()
        .map(|(l, n)| format!("{l} ({n})"))
        .collect();
    row(
        "languages",
        if langs.is_empty() {
            "none recognized (generic tools still work)".into()
        } else {
            langs.join(", ")
        },
    );
    for cat in [
        CommandCategory::Test,
        CommandCategory::Lint,
        CommandCategory::Typecheck,
        CommandCategory::Build,
    ] {
        row(
            &format!("{} command", cat.label()),
            app.profile
                .first(cat)
                .map(|c| c.run.clone())
                .unwrap_or_else(|| style("not detected").dim().to_string()),
        );
    }

    println!("\n{}", style("Configuration").bold());
    row("config file", display_path(&app.paths.config_file()));
    row("permissions", app.config.permissions.profile.as_str());
    row("telemetry", style("none (Veyra has no telemetry)").green());
    for w in &app.warnings {
        row("warning", style(w).yellow());
    }
}

pub fn doctor(rt: &tokio::runtime::Runtime, opts: &Options) -> anyhow::Result<i32> {
    let app = App::load(opts)?;
    print_doctor(&app, rt);
    Ok(0)
}

pub fn print_privacy(config: &Config) {
    let r = PrivacyReport::from_config(config);
    for (k, v) in r.lines() {
        let v = if v == "disabled" || v == "no" || v.starts_with("local") || v == "127.0.0.1" {
            style(v).green().to_string()
        } else if v == "ENABLED" || v == "yes" || v.starts_with("REMOTE") {
            style(v).red().bold().to_string()
        } else {
            v
        };
        println!("{:<24}{}", format!("{k}:"), v);
    }
    println!("\nNetwork is used only for:");
    for u in &r.network_uses {
        println!("  - {u}");
    }
    println!("Inference requests go to the model endpoint above and nowhere else.");
}

pub fn privacy(opts: &Options) -> anyhow::Result<i32> {
    let paths = veyra_core::VeyraPaths::discover()?;
    let root = crate::app::workspace_root(opts).ok();
    let loaded = Config::load(&paths.config_file(), root.as_deref())?;
    for w in &loaded.warnings {
        eprintln!("warning: {w}");
    }
    print_privacy(&loaded.config);
    Ok(0)
}
