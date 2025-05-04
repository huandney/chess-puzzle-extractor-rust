// src/generator.rs
// ---------------------------------------------------------------------------
// Fase 1: coleta de candidatos  •  Fase 2: geração de puzzles completos
// ---------------------------------------------------------------------------

use std::{path::PathBuf, time::Instant};

use anyhow::Result;
use log::{info, warn};

use crate::{
    builder::{create_puzzle_tree, process_puzzle},
    candidates::{CandidateContext, PuzzleCandidate},
    engine::Engine,
    exporter::export_puzzle,
    resume::{initialize_resume, update_resume_data},
    utils::{iterate_games, prepare_engine, prepare_output_file},
};

pub struct GeneratorArgs { pub base_depth: u8, pub resume: bool, pub verbose: bool }

pub struct GenerateResult { puzzles: u64 }
impl GenerateResult { pub fn total(&self) -> u64 { self.puzzles } }

pub async fn generate_puzzles(
    input: &PathBuf,
    output: Option<&PathBuf>,
    args: GeneratorArgs,
) -> Result<GenerateResult> {
    let t0 = Instant::now();

    // recursos
    let (out_path, mut out_file) = prepare_output_file(input, output, args.resume)?;
    let (depths, mut engine)     = prepare_engine(args.base_depth).await?;
    let (_, _, stats) =
        initialize_resume(input, out_path.parent().unwrap().to_str().unwrap(), args.resume)?;

    // ---------- fase 1: candidatos ---------------------------------------
    let mut ctx = CandidateContext { engine: &mut engine, progress_bar: None };
    let mut pool: Vec<(PuzzleCandidate, Vec<(String, String)>)> = Vec::new();

    for rec in iterate_games(input)? {
        // avaliação pré‑blunder (rápida)
        let prev_std = ctx.engine.analyze(&rec.board, depths.scan, 1).await?
            .get(0).and_then(|i| i.score.clone());
        let Some(std) = prev_std else { continue };
        let prev_cp = Engine::to_cp(&std);

        // tenta criar candidato
        if let Ok(Some(c)) = ctx
            .find_candidate(
                &rec.board,
                &rec.mv,
                prev_cp,
                &depths,
                rec.move_idx,
            )
            .await
        {
            pool.push((c, rec.headers.clone()));
        }
    }
    info!("fase‑1 concluída → {} candidatos", pool.len());

    // ---------- fase 2: puzzles ------------------------------------------
    let mut total = 0u64;

    for (cand, hdrs) in pool {
        let Some(seq) = create_puzzle_tree(
            &mut engine,
            &cand.board_post_blunder,
            cand.solver_color,
            cand.pre_cp,
            &depths,
        )
        .await?
        else { continue };

        if let Ok(pgn) = process_puzzle(&cand, &seq, &hdrs) {
            if export_puzzle(&pgn, &mut out_file).is_ok() { total += 1; }
        }
    }

    // resume
    if let Err(e) = update_resume_data(
        input,
        0,
        &stats,
        out_path.parent().unwrap().to_str().unwrap(),
    ) {
        warn!("resume update falhou: {e}");
    }

    info!("finalizado: {total} puzzles em {:.2?}", t0.elapsed());
    Ok(GenerateResult { puzzles: total })
}
