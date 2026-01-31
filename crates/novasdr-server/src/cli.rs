use clap::Parser;
use std::path::PathBuf;

#[derive(Debug, clap::Subcommand, Clone)]
pub enum Command {
    /// Interactive configuration wizard (writes JSON config files).
    Setup,
    /// Edit existing configuration with an interactive wizard.
    Configure,
    /// Run benchmark
    Benchmark {
        kind: BenchmarkKind,
        iterations: Option<usize>,
        fftsize: Option<usize>,
    },
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum BenchmarkKind {
    CpuFftComplex,
    CpuFftReal,
    ClFftComplex,
    ClFftReal,
    VkFftComplex,
    VkFftReal,
    Ssb,
}

#[derive(Debug, Parser)]
#[command(name = "novasdr-server", version, about)]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Command>,

    #[arg(
        short = 'c',
        long = "config",
        default_value = "config/config.json",
        global = true
    )]
    pub config: PathBuf,
    #[arg(
        short = 'r',
        long = "receivers",
        default_value = "config/receivers.json",
        global = true
    )]
    pub receivers: PathBuf,
    #[arg(short = 'd', long = "debug")]
    pub debug: bool,
    #[arg(long = "log-dir")]
    pub log_dir: Option<PathBuf>,
    #[arg(long = "no-file-log")]
    pub no_file_log: bool,
}
