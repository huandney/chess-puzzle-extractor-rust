// src/main.rs
// ---------------------------------------------------------------------------
// CLI simples para extrair puzzles.
// ---------------------------------------------------------------------------

use std::{fs, path::PathBuf, process::Command};

use anyhow::{Context, Result};
use clap::Parser;
use log::{info, error};

mod analysis;
mod builder;
mod candidates;
mod config;
mod engine;
mod exporter;
mod generator;
mod resume;
mod statistics;
mod utils;
mod visual;

/// Args CLI - Argumentos da linha de comando para configuração
#[derive(Parser, Debug)]
#[command(author, version, about)]
pub struct Args {
    pub input: PathBuf,                                       // Arquivo PGN de entrada
    #[arg(short, long)]
    pub output: Option<PathBuf>,                              // Saída opcional (ou usa padrão)
    #[arg(short, long, default_value_t = config::DEFAULT_DEPTH)]
    pub depth: u8,                                            // Profundidade de análise
    #[arg(short, long)]
    pub resume: bool,                                         // Flag para retomar processamento
    #[arg(short, long)]
    pub verbose: bool,                                        // Verbosidade
    #[arg(long, default_value = "info")]
    pub log_level: String,                                    // Nível de logging
}

/// Configura o logger com o nível especificado
fn setup_logger(level:&str){ env_logger::Builder::new().filter_level(level.parse().unwrap_or(log::LevelFilter::Info)).init(); }

/// Verifica se o Stockfish está disponível localmente ou no PATH
fn ensure_stockfish() -> Result<()> {
    if fs::metadata("./stockfish").is_ok() || Command::new("stockfish").arg("--version").output().is_ok() {
        return Ok(());
    }
    error!("Stockfish não encontrado");
    anyhow::bail!("Stockfish ausente");
}

/// Ponto de entrada principal do programa
#[tokio::main]
async fn main() -> Result<()> {
    // Parse argumentos e configura logger
    let args = Args::parse();
    setup_logger(&args.log_level);

    // Verifica disponibilidade do Stockfish
    ensure_stockfish()?;

    // Prepara argumentos para o gerador
    let gen_args = generator::GeneratorArgs { base_depth: args.depth, resume: args.resume, verbose: args.verbose };

    // Executa o gerador de puzzles
    let result = generator::generate_puzzles(&args.input, args.output.as_ref(), gen_args)
        .await
        .context("erro gerando puzzles")?;

    // Exibe resultado
    info!("puzzles gerados: {}", result.total());
    Ok(())
}
