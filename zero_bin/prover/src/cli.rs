use clap::Args;

const HELP_HEADING: &str = "Prover options";

/// Represents the main configuration structure for the runtime.
#[derive(Args, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct CliProverConfig {
    /// The log of the max number of CPU cycles per proof.
    #[arg(short, long, help_heading = HELP_HEADING, default_value_t = 20)]
    max_cpu_len_log: usize,
    /// Number of transactions in a batch to process at once.
    #[arg(short, long, help_heading = HELP_HEADING, default_value_t = 1)]
    batch_size: usize,
    /// If true, save the public inputs to disk on error.
    #[arg(short='i', long, help_heading = HELP_HEADING, default_value_t = false)]
    save_inputs_on_error: bool,
}

impl Default for CliProverConfig {
    fn default() -> Self {
        Self {
            max_cpu_len_log: 20,
            batch_size: 1,
            save_inputs_on_error: false
        }
    }
}

impl From<CliProverConfig> for crate::ProverConfig {
    fn from(cli: CliProverConfig) -> Self {
        Self {
            batch_size: cli.batch_size,
            max_cpu_len_log: cli.max_cpu_len_log,
            save_inputs_on_error: cli.save_inputs_on_error,
        }
    }
}
