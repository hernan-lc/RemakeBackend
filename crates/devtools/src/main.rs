mod cli;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command, DatabaseCommand};
use devtools_core::build;
use devtools_core::database::{DatabaseStatus, InstallOptions, InstallSource};
use devtools_core::dev::{self, DevOptions};
use devtools_core::report::{self, Level};
use devtools_core::tests::{exit_code_for, Status, TestRun};
use devtools_core::{clean, DevConfig, DevEnvironment};
use serde::Serialize;
use serde_json::json;
use std::fs;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let json = cli.json;
    match run(cli).await {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            if json {
                println!("{}", json!({ "error": format!("{e:#}") }));
            } else {
                eprintln!("error: {e:#}");
            }
            std::process::exit(1);
        }
    }
}

fn emit<T: Serialize>(as_json: bool, value: &T, human: impl FnOnce() -> String) -> Result<()> {
    if as_json {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        println!("{}", human());
    }
    Ok(())
}

fn status_line(s: &DatabaseStatus) -> String {
    format!(
        "pgrust: {}\n  installed: {}{}\n  port: {}\n  pid: {}\n  healthy: {}\n  server: {}\n  data: {} ({})\n  binary: {}",
        if s.running { "running" } else { "stopped" },
        s.installed,
        s.installed_version.as_deref().map(|v| format!(" ({v})")).unwrap_or_default(),
        s.port,
        s.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".into()),
        s.healthy.map(|h| h.to_string()).unwrap_or_else(|| "-".into()),
        s.server_version.as_deref().unwrap_or("-"),
        s.data_dir.display(),
        if s.data_initialized { "initialized" } else { "not created yet" },
        s.binary_path.display(),
    )
}

fn runs_summary(runs: &[TestRun]) -> String {
    runs.iter()
        .map(|r| {
            format!(
                "{}: {:?} — {} passed, {} failed, {} skipped ({} ms){}",
                r.kind,
                r.status,
                r.passed,
                r.failed,
                r.skipped,
                r.duration_ms,
                r.note
                    .as_deref()
                    .map(|n| format!("\n  {n}"))
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn install_source(
    source: Option<std::path::PathBuf>,
    prebuilt: Option<std::path::PathBuf>,
) -> InstallSource {
    match prebuilt {
        Some(dir) => InstallSource::Prebuilt(dir),
        None => InstallSource::Source(source.unwrap_or_else(|| "vendor/pgrust".into())),
    }
}

async fn run(cli: Cli) -> Result<i32> {
    let json = cli.json;
    let env = DevEnvironment::load()?;
    match cli.command {
        Command::Setup => {
            env.ensure_dirs()?;
            let mut created = Vec::new();
            let config_path = env.root.join(devtools_core::config::CONFIG_FILE);
            if !config_path.is_file() {
                fs::write(&config_path, toml::to_string_pretty(&DevConfig::default())?)?;
                created.push(config_path.display().to_string());
            }
            let ignore = env.root.join(".gitignore");
            let text = fs::read_to_string(&ignore).unwrap_or_default();
            if !text
                .lines()
                .any(|l| l.trim() == "/.devtools/" || l.trim() == ".devtools/")
            {
                let sep = if text.is_empty() || text.ends_with('\n') {
                    ""
                } else {
                    "\n"
                };
                fs::write(&ignore, format!("{text}{sep}/.devtools/\n"))?;
                created.push(format!("{} (added /.devtools/)", ignore.display()));
            }
            let doctor = report::doctor(&env).await;
            emit(
                json,
                &json!({ "created": created, "doctor": doctor }),
                || {
                    let mut out = if created.is_empty() {
                        "nothing to create".to_string()
                    } else {
                        format!("created:\n  {}", created.join("\n  "))
                    };
                    out.push_str(&format!(
                        "\nnext: remake-dev db install   (then: db up, dev, test)\ndoctor: {}",
                        if doctor.ok {
                            "ok"
                        } else {
                            "problems found; run `remake-dev doctor`"
                        }
                    ));
                    out
                },
            )?;
            Ok(0)
        }
        Command::Dev {
            ui,
            no_watch,
            no_db,
            no_migrate,
        } => {
            let watch = if no_watch { Some(false) } else { None };
            dev::run(
                &env,
                &DevOptions {
                    ui,
                    watch,
                    no_db,
                    no_migrate,
                },
            )
            .await?;
            Ok(0)
        }
        Command::Build { release } => {
            let record = build::build_backend(&env, release)?;
            emit(json, &record, || {
                format!("build {:?} in {} ms", record.status, record.duration_ms)
            })?;
            Ok(if record.status == Status::Passed {
                0
            } else {
                1
            })
        }
        Command::Check { clippy } => {
            let record = build::check_backend(&env, clippy)?;
            emit(json, &record, || {
                format!(
                    "{} {:?} in {} ms",
                    record.kind, record.status, record.duration_ms
                )
            })?;
            Ok(if record.status == Status::Passed {
                0
            } else {
                1
            })
        }
        Command::Test {
            unit,
            integration,
            all,
        } => {
            let mut runner = env.tests();
            runner.quiet_stdout = json;
            let runs = if all {
                runner.run_all().await?
            } else if integration {
                let mut runs = Vec::new();
                if unit {
                    runs.push(runner.run_unit()?);
                }
                runs.push(runner.run_integration().await?);
                runs
            } else {
                vec![runner.run_unit()?]
            };
            let code = exit_code_for(&runs);
            emit(json, &json!({ "runs": runs, "exit_code": code }), || {
                runs_summary(&runs)
            })?;
            Ok(code)
        }
        Command::Doctor => {
            let doctor = report::doctor(&env).await;
            emit(json, &doctor, || {
                doctor
                    .checks
                    .iter()
                    .map(|c| {
                        let mark = match c.level {
                            Level::Ok => "ok  ",
                            Level::Info => "info",
                            Level::Warn => "warn",
                            Level::Fail => "FAIL",
                        };
                        format!("[{mark}] {}: {}", c.name, c.detail)
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })?;
            Ok(if doctor.ok { 0 } else { 1 })
        }
        Command::Report { format } => {
            let generator = env.reports();
            let report = generator.generate().await?;
            let markdown = !json && matches!(format.as_str(), "markdown" | "md");
            let path = generator.save(&report, if markdown { "markdown" } else { "json" })?;
            if markdown {
                println!("{}", report::to_markdown(&report));
            } else {
                println!("{}", serde_json::to_string_pretty(&report)?);
            }
            eprintln!("saved to {}", path.display());
            Ok(0)
        }
        Command::Clean => {
            let outcome = clean::clean(&env)?;
            emit(json, &outcome, || {
                format!(
                    "removed {} generated file(s); kept the install, the data, the pid and the server log",
                    outcome.removed.len()
                )
            })?;
            Ok(0)
        }
        Command::Db { command } => {
            let db = env.database();
            match command {
                DatabaseCommand::Install {
                    source,
                    prebuilt,
                    force,
                    slim,
                } => {
                    let outcome = db.install(&InstallOptions {
                        source: install_source(source, prebuilt),
                        force,
                        upgrade: false,
                        slim,
                        features: None,
                        accept_data_risk: false,
                    })?;
                    emit(json, &outcome, || {
                        format!(
                            "installed {} (sha256 {}…) at {}",
                            outcome.manifest.version,
                            &outcome.manifest.binary_sha256[..12],
                            env.install_dir().display()
                        )
                    })?;
                }
                DatabaseCommand::Upgrade {
                    source,
                    prebuilt,
                    slim,
                    accept_data_risk,
                } => {
                    let outcome = db.install(&InstallOptions {
                        source: install_source(source, prebuilt),
                        force: true,
                        upgrade: true,
                        slim,
                        features: None,
                        accept_data_risk,
                    })?;
                    emit(json, &outcome, || {
                        format!(
                            "upgraded {} -> {}",
                            outcome
                                .replaced
                                .as_ref()
                                .map(|m| m.version.as_str())
                                .unwrap_or("?"),
                            outcome.manifest.version
                        )
                    })?;
                }
                DatabaseCommand::Up => {
                    let up = db.up().await?;
                    emit(json, &up, || {
                        format!(
                            "{}\n{}\nlog: {}\nDATABASE_URL=postgres://{}:<password>@{}:{}/{}",
                            if up.reused {
                                "reused the running instance"
                            } else {
                                "started"
                            },
                            status_line(&up.status),
                            up.log_file.display(),
                            env.conn.user,
                            env.conn.host,
                            env.conn.port,
                            env.conn.database
                        )
                    })?;
                }
                DatabaseCommand::Down => {
                    let stopped = db.down().await?;
                    emit(json, &json!({ "stopped": stopped }), || {
                        if stopped {
                            "pgrust stopped; data retained".into()
                        } else {
                            "no managed pgrust was running".into()
                        }
                    })?;
                }
                DatabaseCommand::Restart => {
                    let up = db.restart().await?;
                    emit(json, &up, || {
                        format!("restarted\n{}", status_line(&up.status))
                    })?;
                }
                DatabaseCommand::Status => {
                    let status = db.status().await?;
                    emit(json, &status, || status_line(&status))?;
                }
                DatabaseCommand::Logs { lines } => {
                    let text = db.logs(lines)?;
                    emit(json, &json!({ "log": text }), || text.clone())?;
                }
                DatabaseCommand::Reset { yes } => {
                    let name = db.reset_test_database(yes).await?;
                    emit(json, &json!({ "reset": name }), || {
                        format!("recreated the isolated test database `{name}`")
                    })?;
                }
            }
            Ok(0)
        }
    }
}
