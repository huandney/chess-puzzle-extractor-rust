// src/resume.rs
// Gerencia persistência do progresso para permitir retomada de processamento

// Biblioteca padrão
use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};

// Bibliotecas externas
use anyhow::{Context, Result};
use serde_json::{json, Value};

// Módulos internos
use crate::statistics::PuzzleStatistics;

/// Obtém o caminho do arquivo de resumo para o PGN dado
pub fn get_resume_file(input_path: &Path, puzzles_dir: &str) -> PathBuf {
    let base_name = input_path.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let resume_dir = PathBuf::from(puzzles_dir).join(".resume");
    fs::create_dir_all(&resume_dir).unwrap_or_default();

    resume_dir.join(format!("{}.json", base_name))
}

/// Carrega dados de resumo (se existir) para o PGN dado
pub fn load_resume(input_path: &Path, puzzles_dir: &str) -> Option<Value> {
    let resume_file = get_resume_file(input_path, puzzles_dir);
    if resume_file.exists() {
        let file = File::open(&resume_file).ok()?;
        let reader = BufReader::new(file);
        serde_json::from_reader(reader).ok()
    } else {
        None
    }
}

/// Inicializa dados de resumo para o início da análise
pub fn initialize_resume(
    input_path: &Path,
    puzzles_dir: &str,
    resume_flag: bool
) -> Result<(Value, u64, PuzzleStatistics)> {
    if !resume_flag {
        // Criar novos dados para uma nova análise
        let resume_data = json!({
            "games_analyzed": 0,
            "elapsed_time": 0,
            "stats": {
                "total_games": 0,
                "puzzles_found": 0,
                "puzzles_rejected": 0,
                "objective_stats": {},
                "phase_stats": {},
                "rejection_reasons": {}
            }
        });

        save_resume(input_path, &resume_data, puzzles_dir)?;
        let games_analyzed = 0;
        let stats = PuzzleStatistics::new();

        Ok((resume_data, games_analyzed, stats))
    } else {
        // Carregar dados existentes
        let resume_data = load_resume(input_path, puzzles_dir)
            .ok_or_else(|| anyhow::anyhow!("Falha ao carregar dados de resume"))?;
        let games_analyzed = resume_data.get("games_analyzed")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        // Criar estatísticas a partir dos dados carregados
        let stats = PuzzleStatistics::from_resume_data(&resume_data);

        Ok((resume_data, games_analyzed, stats))
    }
}

/// Salva dados de resumo para o PGN dado
pub fn save_resume(
    input_path: &Path,
    data: &Value,
    puzzles_dir: &str,
) -> Result<()> {
    let resume_file = get_resume_file(input_path, puzzles_dir);
    let file = File::create(&resume_file).context("Falha ao criar arquivo de resumo")?;
    serde_json::to_writer_pretty(file, data).context("Falha ao gravar dados de resumo")?;
    Ok(())
}

/// Atualiza os dados de resumo com estatísticas e contagem de jogos processados
pub fn update_resume_data(
    input_path: &Path,
    games_analyzed: u64,
    stats: &PuzzleStatistics,
    puzzles_dir: &str
) -> Result<()> {
    let resume_data = json!({
        "games_analyzed": games_analyzed,
        "elapsed_time": stats.get_elapsed_time(),
        "stats": stats
    });
    save_resume(input_path, &resume_data, puzzles_dir)?;
    Ok(())
}

/// Pula os primeiros `n` elementos de qualquer iterador.
pub fn skip_processed_games<T, I>(
    iter: I,
    games_analyzed: usize,
) -> impl Iterator<Item = T>
where
    I: Iterator<Item = T>,
{
    iter.skip(games_analyzed)
}
