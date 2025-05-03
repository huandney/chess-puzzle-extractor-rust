// src/statistics.rs
// Coleta e gerencia estatísticas sobre o processo de geração de puzzles

// Biblioteca padrão
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

// Bibliotecas externas
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PuzzleStatistics {
    // Dados de tempo
    #[serde(skip)]
    start_time: Option<Instant>,
    pub elapsed_secs: u64,

    // Contadores principais
    pub total_games: u64,
    pub puzzles_found: u64,
    pub puzzles_rejected: u64,

    // Estatísticas detalhadas
    pub objective_stats: HashMap<String, u64>,
    pub phase_stats: HashMap<String, u64>,
    pub rejection_reasons: HashMap<String, u64>,
}

impl PuzzleStatistics {
    pub fn new() -> Self {
        let mut stats = Self::default();
        stats.start_time = Some(Instant::now());
        stats
    }

    pub fn from_resume_data(resume_data: &serde_json::Value) -> Self {
        let stats = if let Some(stats) = resume_data.get("stats") {
            // Tentar converter o JSON para a estrutura
            serde_json::from_value(stats.clone()).unwrap_or_default()
        } else {
            Self::default()
        };

        // Inicializa o tempo na retomada
        let mut result = stats;
        result.start_time = Some(Instant::now());

        // Carrega tempo decorrido do arquivo se disponível
        if let Some(elapsed) = resume_data.get("elapsed_time") {
            if let Some(secs) = elapsed.as_u64() {
                result.elapsed_secs = secs;
            }
        }

        result
    }

    pub fn increment_games(&mut self, count: u64) {
        self.total_games += count;
    }

    pub fn add_found(&mut self, count: u64) {
        self.puzzles_found += count;
    }

    pub fn add_rejected(&mut self, reason: &str, count: u64) {
        self.puzzles_rejected += count;
        *self.rejection_reasons.entry(reason.to_string()).or_insert(0) += count;
    }

    // Atualiza estatísticas de objetivos (motivos táticos) e fases do jogo dos puzzles
    pub fn update_objective(&mut self, objective: &str, count: u64) {
        *self.objective_stats.entry(objective.to_string()).or_insert(0) += count;
    }

    pub fn update_phase(&mut self, phase: &str, count: u64) {
        *self.phase_stats.entry(phase.to_string()).or_insert(0) += count;
    }

    pub fn get_elapsed_time(&self) -> u64 {
        let current = self.start_time.map(|t| t.elapsed().as_secs()).unwrap_or(0);
        self.elapsed_secs + current
    }

    pub fn get_average_time_per_game(&self) -> f64 {
        if self.total_games == 0 {
            0.0
        } else {
            self.get_elapsed_time() as f64 / self.total_games as f64
        }
    }
}

#[derive(Debug)]
pub struct AnalysisResult {
    pub total_games: u64,
    pub puzzles_found: u64,
    pub puzzles_rejected: u64,
    pub rejection_reasons: HashMap<String, u64>,
    pub was_interrupted: bool,
    pub elapsed_time: u64,
    pub avg_time_per_game: f64,
    pub stats: PuzzleStatistics,
}

impl AnalysisResult {
    pub fn new(stats: PuzzleStatistics, was_interrupted: bool) -> Self {
        AnalysisResult {
            total_games: stats.total_games,
            puzzles_found: stats.puzzles_found,
            puzzles_rejected: stats.puzzles_rejected,
            rejection_reasons: stats.rejection_reasons.clone(),
            was_interrupted,
            elapsed_time: stats.get_elapsed_time(),
            avg_time_per_game: stats.get_average_time_per_game(),
            stats,
        }
    }

    pub fn successful(&self) -> bool {
        !self.was_interrupted
    }

    pub fn display_statistics(&self, output_path: Option<&Path>) -> Result<()> {
        crate::visual::render_end_statistics(
            self.total_games,
            self.puzzles_found,
            self.puzzles_rejected,
            self.elapsed_time,
            self.avg_time_per_game,
            &self.rejection_reasons,
            &self.stats.objective_stats,
            &self.stats.phase_stats,
            output_path,
        )?;
        Ok(())
    }
}
