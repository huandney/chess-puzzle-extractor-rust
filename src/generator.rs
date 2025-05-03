// src/generator.rs
// ---------------------------------------------------------------------------
// Fase 1: varre PGN com candidates::find_candidate e armazena cada
//         PuzzleCandidate em um Vec.
// Fase 2: resolve cada candidato e exporta o puzzle.
// ---------------------------------------------------------------------------

use std::{path::PathBuf, time::Instant};

use anyhow::Result;
use log::{info, warn, error};

use crate::{
    builder::{create_puzzle_tree, process_puzzle},
    candidates::{find_candidate, CandidateContext, PuzzleCandidate},
    exporter::export_puzzle,
    resume::{initialize_resume, update_resume_data},
    utils::{iterate_games, prepare_engine, prepare_output_file, DepthSet},
};

/// Parâmetros mínimos para geração de puzzles
pub struct GeneratorArgs {
    pub base_depth: u8,    // Profundidade base de análise
    pub resume:     bool,  // Flag para retomar processamento anterior
    pub verbose:    bool,  // Controla saída verbosa
}

/// Resultado simples da geração
pub struct GenerateResult { puzzles: u64 }
impl GenerateResult { pub fn total(&self) -> u64 { self.puzzles } }

/// Orquestra o processo completo de extração de puzzles em duas fases
pub async fn generate_puzzles(
    input:  &PathBuf,         // Arquivo PGN de entrada
    output: Option<&PathBuf>, // Arquivo de saída opcional (ou usa padrão)
    args:   GeneratorArgs,    // Argumentos de configuração
) -> Result<GenerateResult> {
    // Inicializa timer e recursos
    let t0 = Instant::now();
    let (out_path, mut out_file) = prepare_output_file(input, output, args.resume)?;
    let (depths, mut engine)     = prepare_engine(args.base_depth).await?;

    // Inicializa estatísticas (apenas para manter compatibilidade com resume)
    let (_, _, mut stats) =
        initialize_resume(input, out_path.parent().unwrap().to_str().unwrap(), args.resume)?;

    // ---------- fase 1 – coleta de candidatos rápidos ---------------------
    // Armazena todos os candidatos em memória para processamento posterior
    let mut candidates = Vec::<PuzzleCandidate>::new();

    // Varre o PGN em busca de blunders que possam gerar puzzles
    for rec in iterate_games(input)? {
        // Tenta criar contexto de candidato para o lance atual
        let ctx = match CandidateContext::for_blunder(
            &mut engine,
            &rec.board,
            &rec.mv,
            &depths,
            None,
            rec.move_idx,
        )
        .await
        {
            Ok(c) => c,
            Err(_) => continue,  // Falha: pula para o próximo lance
        };

        // Se for um candidato válido, adiciona à lista
        if let Ok(Some(c)) = find_candidate(ctx).await { candidates.push(c); }
    }
    info!("coleta concluída → {} candidatos", candidates.len());

    // ---------- fase 2 – análise e exportação ----------------------------
    // Processa cada candidato coletado para gerar puzzles completos
    let mut puzzles = 0u64;

    for cand in candidates {
        // Tenta criar árvore completa de lances para o puzzle
        let seq = match create_puzzle_tree(
            &mut engine,
            &cand.board_post_blunder,
            cand.solver_color,
            cand.pre_cp,
            &depths,
        )
        .await
        {
            Ok(Some(s)) => s,
            _           => continue,  // Falha: pula para o próximo candidato
        };

        // Processa e exporta o puzzle
        if let Ok(pgn) = process_puzzle(&cand, &seq) {
            if export_puzzle(&pgn, &mut out_file).is_ok() {
                puzzles += 1;
            }
        }
    }

    // Atualiza informações de resume (apenas contagem básica)
    if let Err(e) = update_resume_data(
        input,
        0,
        &stats,
        out_path.parent().unwrap().to_str().unwrap(),
    ) {
        warn!("resume update falhou: {e}");
    }

    // Resultado final
    info!("finalizado: {puzzles} puzzles em {:.2?}", t0.elapsed());
    Ok(GenerateResult { puzzles })
}
