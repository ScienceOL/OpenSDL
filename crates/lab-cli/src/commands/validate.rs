use std::path::PathBuf;

use clap::Args;

#[derive(Debug, Args)]
pub struct ValidateArgs {
    /// Portable virtual-asset directory containing asset.yaml.
    pub directory: PathBuf,
    /// Emit a machine-readable dynamic conformance report.
    #[arg(long)]
    pub json: bool,
}

pub fn run(args: ValidateArgs) -> anyhow::Result<()> {
    let report = osdl_server::simulation_validation::validate_directory(&args.directory)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if let Some(dynamic) = report.dynamic {
        println!(
            "{} {} ({}): {} actions, {} observations, {:.2} simulated seconds",
            report.name,
            report.version,
            dynamic.engine,
            dynamic.actions_executed,
            dynamic.observations_checked,
            dynamic.simulated_seconds
        );
    } else if let Some(runtime) = report.runtime {
        println!(
            "{} {} ({runtime} runtime admitted; no instrument model conformance)",
            report.name, report.version
        );
    } else {
        println!(
            "{} {} (static asset; no runtime)",
            report.name, report.version
        );
    }
    Ok(())
}
