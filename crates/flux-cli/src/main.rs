mod exit_codes;
mod error;
mod diagnostics;
mod formatter;
mod csv_loader;
mod data;
mod interpreter;
mod math_builtins;
mod stat_indicators;
mod portfolio_ops;
mod commands;

use std::path::PathBuf;
use std::process;

use clap::error::ErrorKind;
use clap::{Parser, Subcommand};

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
        /// Path to the CSV data file
        #[arg(long)]
        data: PathBuf,
        /// Initial capital for portfolio tracking (default: 10000)
        #[arg(long, default_value = "10000.0")]
        capital: f64,
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
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            err.print().expect("failed to write error");
            match err.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => process::exit(SUCCESS),
                _ => process::exit(USAGE_ERROR),
            }
        }
    };

    let exit_code = match cli.command {
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
        Commands::Backtest { file, data, capital } => {
            match commands::backtest::run_backtest_cmd(&file, &data, capital) {
                Ok(()) => SUCCESS,
                Err(_e) => FAILURE,
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
                process::exit(USAGE_ERROR);
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
    };

    process::exit(exit_code);
}
