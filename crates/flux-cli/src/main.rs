mod exit_codes;
mod error;
mod diagnostics;
mod formatter;
mod csv_loader;
mod data;
mod interpreter;
mod math_builtins;
mod module_resolver;
mod stat_indicators;
mod portfolio_ops;
mod live;
mod commands;

use std::path::PathBuf;
use std::process;

use clap::error::ErrorKind;
use clap::{Parser, Subcommand};

use commands::live::LiveArgs;
use commands::nucleus::NucleusArgs;
use exit_codes::{FAILURE, SUCCESS, USAGE_ERROR};

#[derive(Parser)]
#[command(name = "flux", version, about = "The Flux language CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Check a Flux source file for errors
    Check {
        /// Path to the Flux source file
        file: PathBuf,
    },
    /// Build a Flux source file and emit generated Rust code
    Build {
        /// Path to the Flux source file
        file: PathBuf,
        /// Output file path for generated code
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Backtest a Flux strategy against CSV data
    Backtest {
        /// Path to the Flux source file
        file: PathBuf,
        /// Path to CSV data file(s) — repeat for multiple symbols
        #[arg(long, required = true)]
        data: Vec<PathBuf>,
        /// Initial capital for portfolio tracking (default: 10000)
        #[arg(long, default_value = "10000.0")]
        capital: f64,
        /// Engine fidelity level (0=fast, 1=synthetic, 2=replay)
        #[arg(long, default_value = "0")]
        fidelity: u8,
        /// Number of price levels per side (fidelity 1 only, range: 1-20)
        #[arg(long)]
        depth: Option<u32>,
        /// Spread percentage between levels (fidelity 1 only, range: 0.01-10.0)
        #[arg(long)]
        spread: Option<f64>,
        /// Total liquidity per side (fidelity 1 only, range: 100-10000000)
        #[arg(long)]
        liquidity: Option<f64>,
        /// Path to L2 data file (required for fidelity 2)
        #[arg(long)]
        l2_data: Option<PathBuf>,
        /// Per-symbol contract multiplier (point value) as SYMBOL:VALUE pairs
        /// e.g., --multiplier "ES=F:50,NQ=F:20,RTY=F:50,YM=F:5"
        #[arg(long)]
        multiplier: Option<String>,
    },
    /// Initialize a new Flux project
    Init {
        /// Project name (defaults to current directory name)
        name: Option<String>,
    },
    /// Format a Flux source file with optional colorization
    Fmt {
        /// Path to the Flux source file
        file: PathBuf,
        /// Force color output even when not a TTY
        #[arg(long)]
        color: bool,
        /// Disable color output
        #[arg(long)]
        no_color: bool,
        /// Reformat the file in place
        #[arg(long)]
        write: bool,
        /// Check if file needs formatting (exit 1 if yes)
        #[arg(long)]
        check: bool,
    },
    /// Fetch historical market data from a provider
    Fetch {
        /// One or more stock symbols (comma-separated)
        symbols: String,
        /// Data provider (default: yahoo)
        #[arg(long, default_value = "yahoo")]
        source: String,
        /// Relative time period (e.g., 1y, 6mo, 5d)
        #[arg(long)]
        period: Option<String>,
        /// Bar interval (e.g., 1d, 1h, 5m)
        #[arg(long, default_value = "1d")]
        interval: String,
        /// Start date (YYYY-MM-DD)
        #[arg(long)]
        from: Option<String>,
        /// End date (YYYY-MM-DD)
        #[arg(long)]
        to: Option<String>,
        /// Output file path (default: stdout)
        #[arg(long, short)]
        output: Option<PathBuf>,
    },
    /// Run a strategy end-to-end: compile, fetch data, and backtest
    Run {
        /// Path to the Flux source file
        file: PathBuf,
        /// Override symbols (comma-separated)
        #[arg(long)]
        symbols: Option<String>,
        /// Override time period (e.g., 1y, 6mo)
        #[arg(long)]
        period: Option<String>,
        /// Override bar interval (e.g., 1d, 1h)
        #[arg(long)]
        interval: Option<String>,
        /// Data provider (default: from data block or "yahoo")
        #[arg(long)]
        source: Option<String>,
        /// Initial capital for portfolio tracking (default: 10000)
        #[arg(long, default_value = "10000.0")]
        capital: f64,
    },
    /// Run strategies continuously against live market data
    Live(LiveArgs),
    /// Hypothesis-driven strategy development framework
    Nucleus(NucleusArgs),
}

fn main() {
    let exit_code = run();
    process::exit(exit_code);
}

fn run() -> i32 {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            err.print().expect("failed to write error");
            match err.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => return SUCCESS,
                _ => return USAGE_ERROR,
            }
        }
    };

    match cli.command {
        Commands::Check { file } => match commands::check::run_check(&file) {
            Ok(()) => SUCCESS,
            Err(_e) => FAILURE,
        },
        Commands::Build { file, output } => {
            match commands::build::run_build(&file, output.as_deref()) {
                Ok(()) => SUCCESS,
                Err(_e) => FAILURE,
            }
        }
        Commands::Backtest { file, data, capital, fidelity, depth, spread, liquidity, l2_data, multiplier } => {
            let data_refs: Vec<&std::path::Path> = data.iter().map(|p| p.as_path()).collect();
            // Parse multiplier string: "ES=F:50,NQ=F:20" → HashMap
            let multipliers = multiplier.as_deref().map(|s| {
                commands::backtest::parse_multipliers(s)
            }).unwrap_or_default();
            match commands::backtest::run_backtest_cmd(&file, &data_refs, capital, fidelity, depth, spread, liquidity, l2_data.as_deref(), &multipliers) {
                Ok(()) => SUCCESS,
                Err(e) => {
                    match &e {
                        error::CliError::Usage(_) => USAGE_ERROR,
                        _ => FAILURE,
                    }
                }
            }
        }
        Commands::Init { name } => match commands::init::run_init(name.as_deref()) {
            Ok(()) => SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                FAILURE
            }
        },
        Commands::Fmt { file, color, no_color, write, check } => {
            // Determine color mode — mutually exclusive flags
            let color_mode = if color && no_color {
                eprintln!("error: flags '--color' and '--no-color' are mutually exclusive");
                return USAGE_ERROR;
            } else if color {
                formatter::ansi::ColorMode::Always
            } else if no_color {
                formatter::ansi::ColorMode::Never
            } else {
                formatter::ansi::ColorMode::Auto
            };

            match commands::fmt::run_fmt(&file, color_mode, write, check) {
                Ok(()) => SUCCESS,
                Err(e) => {
                    match &e {
                        commands::fmt::FmtError::MutuallyExclusive(_, _) => {
                            eprintln!("error: {e}");
                            USAGE_ERROR
                        }
                        commands::fmt::FmtError::FileRead { .. }
                        | commands::fmt::FmtError::FileWrite { .. } => {
                            eprintln!("error: {e}");
                            FAILURE
                        }
                        commands::fmt::FmtError::Compile(_) => {
                            // Diagnostic already printed to stderr by run_fmt
                            FAILURE
                        }
                    }
                }
            }
        },
        Commands::Fetch { symbols, source, period, interval, from, to, output } => {
            match commands::fetch::run_fetch(
                &symbols,
                &source,
                period.as_deref(),
                &interval,
                from.as_deref(),
                to.as_deref(),
                output.as_ref(),
            ) {
                Ok(()) => SUCCESS,
                Err(e) => {
                    eprintln!("{}", e);
                    if e.contains("mutually exclusive")
                        || e.contains("invalid")
                        || e.contains("requires")
                        || e.contains("no symbols")
                    {
                        USAGE_ERROR
                    } else {
                        FAILURE
                    }
                }
            }
        },
        Commands::Run { file, symbols, period, interval, source, capital } => {
            match commands::run::run_run_cmd(
                &file,
                symbols.as_deref(),
                period.as_deref(),
                interval.as_deref(),
                source.as_deref(),
                capital,
            ) {
                Ok(()) => SUCCESS,
                Err(e) => {
                    match &e {
                        error::CliError::Usage(_) => {
                            eprintln!("{}", e);
                            USAGE_ERROR
                        }
                        _ => FAILURE,
                    }
                }
            }
        },
        Commands::Live(args) => {
            let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
            match rt.block_on(commands::live::run_live_cmd(args)) {
                Ok(()) => SUCCESS,
                Err(e) => {
                    eprintln!("error: {e}");
                    FAILURE
                }
            }
        },
        Commands::Nucleus(args) => match commands::nucleus::run_nucleus(args) {
            Ok(()) => SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                FAILURE
            }
        },
    }
}
