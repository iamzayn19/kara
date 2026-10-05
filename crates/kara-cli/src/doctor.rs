//! `kara doctor` and `kara privacy`.

use crate::app::{display_path, App, Options};
use console::style;
use kara_context::CommandCategory;
use kara_core::privacy::PrivacyReport;
use kara_core::Config;
use kara_inference::local::hardware::format_bytes;

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
    let status = kara_inference::local::status(&app.config, &app.paths, &app.models);
    let hw = &status.hardware;
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
        "Memory for models",
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

    println!("\n{}", style("Inference").bold());
    let inf = &app.config.inference;
    row(
        "provider",
        format!("{} ({})", inf.provider.as_str(), inf.provider.label()),
    );
    if inf.provider.is_endpoint() {
        row(
            "endpoint",
            app.config.endpoint().unwrap_or_else(|| "not set".into()),
        );
        row(
            "model",
            if inf.model.is_empty() {
                "first served"
            } else {
                &inf.model
            },
        );
    } else {
        row("model", &inf.model);
    }

    println!("\n{}", style("Local inference (optional)").bold());
    row(
        "runtime",
        status.runtime.clone().unwrap_or_else(|| {
            "not installed (installed only if you choose local inference)".into()
        }),
    );
    if let Some(v) = &status.runtime_version {
        row("version", v);
    }
    if let Some(asset) = &status.runtime_available {
        row("build for this host", asset);
    }
    if status.installed.is_empty() {
        row("installed models", "none");
    }
    for m in &status.installed {
        row(
            "installed model",
            format!(
                "{} ({}, {}, rev {})",
                m.name,
                format_bytes(m.size_bytes),
                m.license,
                &m.revision[..12.min(m.revision.len())]
            ),
        );
    }
    let rec = &status.recommendation;
    row("recommendation", &rec.summary);
    for c in rec.candidates.iter().filter(|c| !c.fits).take(3) {
        row("", style(format!("{}: {}", c.name, c.reason)).dim());
    }
    if rec.model.is_none() {
        for line in kara_inference::source::remote_options().lines() {
            row("", line.trim());
        }
    }

    println!("\n{}", style("Repository").bold());
    row("root", app.root.display());
    row("git", yes_no(kara_context::git::is_repo(&app.root)));
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
    row("permissions", app.config.permissions.mode.as_str());
    row("telemetry", style("none (Kara has no telemetry)").green());
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
    let paths = kara_core::KaraPaths::discover()?;
    let root = crate::app::workspace_root(opts).ok();
    let loaded = Config::load(&paths.config_file(), root.as_deref())?;
    for w in &loaded.warnings {
        eprintln!("warning: {w}");
    }
    print_privacy(&loaded.config);
    Ok(0)
}
