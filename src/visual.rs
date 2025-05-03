//src/visual.rs
// Interface visual e componentes de progresso para o terminal

// Biblioteca padrão
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

// Bibliotecas externas
use anyhow::Result;
use colored::*;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

// Estrutura para manter o estado do console
pub struct Console {
    multi_progress: MultiProgress,
}

impl Console {
    pub fn new() -> Self {
        Console {
            multi_progress: MultiProgress::new(),
        }
    }

    pub fn print(&self, message: &str) {
        println!("{}", message);
    }

    pub fn log(&self, message: &str) {
        eprintln!("{}", message);
    }
}

// Instância global do console
lazy_static::lazy_static! {
    pub static ref CONSOLE: Console = Console::new();
}

// Mensagens coloridas
pub fn console_yellow(message: &str) {
    println!("{}", message.yellow());
}

pub fn print_error(message: &str) {
    println!("{}", message.red().bold());
}

pub fn print_success(message: &str) {
    println!("{}", message.green().bold());
}

// Estrutura para barra de progresso personalizada
pub struct CustomProgressBar {
    progress_bar: ProgressBar,
    elapsed_offset: Arc<AtomicU64>,
}

impl CustomProgressBar {
    pub fn new(total: u64, elapsed_offset_secs: u64) -> Self {
        let pb = ProgressBar::new(total);
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.blue} {msg} [{elapsed_precise}] {wide_bar:.cyan/blue} {pos}/{len} ({eta})")
            .unwrap()
            .progress_chars("#>-"));

        let elapsed_offset = Arc::new(AtomicU64::new(elapsed_offset_secs));

        CustomProgressBar {
            progress_bar: pb,
            elapsed_offset,
        }
    }

    pub fn inc(&self, delta: u64) {
        self.progress_bar.inc(delta);
    }

    pub fn set_message(&self, msg: &str) {
        self.progress_bar.set_message(msg.to_string());
    }

    pub fn finish_with_message(&self, msg: &str) {
        self.progress_bar.finish_with_message(msg.to_string());
    }

    pub fn log(&self, msg: &str) {
        self.progress_bar.println(msg);
    }
}

// Cria uma barra de progresso com offset de tempo
pub fn create_progress(total: u64, elapsed_offset: u64) -> CustomProgressBar {
    CustomProgressBar::new(total, elapsed_offset)
}

// Imprime o cabeçalho principal
pub fn print_main_header() {
    println!("\n{}", "♟️  Extrator de Puzzles de Xadrez".blue().bold());
    println!("{}", "═".repeat(50).cyan());
}

// Imprime informações do Stockfish
pub fn print_stockfish_info(engine_path: &str) {
    println!("{} {}", "Usando Stockfish em:".blue().bold(), engine_path);
}

// Imprime informações de progresso de retomada
pub fn print_resume_info(games_analyzed: u64) {
    println!("{}", format!("Retomando análise a partir do jogo {}...", games_analyzed + 1).green());
}

// Imprime configurações do programa - usando tipo genérico para Args
pub fn print_configurations<T>(args: &T, output_path: &Path)
where
    T: std::fmt::Debug
{
    println!("{}", "⚙️  Configurações:".cyan().bold());
    println!("📤 Saída:           {}", output_path.display().to_string().cyan());
    println!("Argumentos completos: {:?}\n", args);
}

// Informações iniciais de análise
pub fn print_initial_analysis_info(
    input_path: &Path,
    file_size: &str,
    total_games: u64,
    resume: bool,
    games_analyzed: u64,
    depth: u8,
    depths: &std::collections::HashMap<&str, u8>,
    max_variants: u8
) {
    println!("{}", "Iniciando análise tática das partidas...".cyan().bold());
    println!("Arquivo de entrada: {} ({})", input_path.display().to_string().magenta(), file_size.cyan());

    println!("Total de jogos a analisar: {}", total_games.to_string().cyan());

    if resume && games_analyzed > 0 {
        println!("Jogos analisados: {} ({}% concluído)",
            games_analyzed.to_string().green(),
            format!("{:.1}", (games_analyzed as f64 / total_games as f64) * 100.0).cyan());
    }

    if let (Some(&scan), Some(&solve)) = (depths.get("scan"), depths.get("solve")) {
        println!("Profundidade de análise: {} (scan: {}, solve: {})",
            depth, scan.to_string().cyan().bold(), solve.to_string().cyan().bold());
    }

    println!("Variantes máximas permitidas: {}\n", max_variants.to_string().cyan());
}

// Imprime informação do puzzle encontrado
pub fn print_puzzle_found(progress_bar: &CustomProgressBar, puzzles_found: u64, pgn_text: &str) {
    progress_bar.log(&format!("{}", format!("Puzzle #{} Encontrado", puzzles_found).yellow().bold()));
    progress_bar.log(pgn_text);
    progress_bar.log("");
}

// Exibe mensagem detalhada em modo verbose
pub fn print_verbose_puzzle_generated(progress_bar: &CustomProgressBar, message: &str, pgn_text: Option<&str>) {
    progress_bar.log(message);
    if let Some(text) = pgn_text {
        progress_bar.log(text);
        progress_bar.log("");
    }
}

// Versão simplificada para evitar problemas com a interface TUI
pub fn render_end_statistics(
    game_count: u64,
    puzzles_found: u64,
    puzzles_rejected: u64,
    total_time: u64,
    average_time_per_game: f64,
    rejection_reasons: &HashMap<String, u64>,
    objective_stats: &HashMap<String, u64>,
    phase_stats: &HashMap<String, u64>,
    output_path: Option<&Path>,
) -> Result<()> {
    println!("Estatísticas de análise:");
    println!("- Jogos analisados: {}", game_count);
    println!("- Puzzles encontrados: {}", puzzles_found);
    println!("- Puzzles rejeitados: {}", puzzles_rejected);

    let hours = total_time / 3600;
    let minutes = (total_time % 3600) / 60;
    let seconds = total_time % 60;

    println!("- Tempo total: {:02}h {:02}m {:02}s", hours, minutes, seconds);
    println!("- Tempo médio por jogo: {:.2}s", average_time_per_game);

    if !rejection_reasons.is_empty() {
        println!("- Motivos de rejeição:");
        for (reason, count) in rejection_reasons {
            println!("  - {}: {}", reason, count);
        }
    }

    if !objective_stats.is_empty() {
        println!("- Objetivos táticos atingidos:");
        for (obj, count) in objective_stats {
            println!("  - {}: {}", obj, count);
        }
    }

    if !phase_stats.is_empty() {
        println!("- Fases do jogo:");
        for (phase, count) in phase_stats {
            println!("  - {}: {}", phase, count);
        }
    }

    if let Some(path) = output_path {
        println!("\nPuzzles salvos em: {}", path.display());
    }

    Ok(())
}
