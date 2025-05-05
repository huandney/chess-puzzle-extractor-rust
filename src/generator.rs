// src/generator.rs
// ---------------------------------------------------------------------------
// Fase 1: coleta de candidatos  ·  Fase 2: geração de puzzles
// ---------------------------------------------------------------------------

use std::{path::PathBuf, time::Instant};
use anyhow::Result;
use log::info;
use shakmaty::Chess;

use crate::{
    builder::{create_puzzle_tree, process_puzzle},
    candidates::CandidateContext,
    exporter::export_puzzle,
    resume::{initialize_resume, update_resume_data},
    engine::Engine,
    utils::{iterate_games, prepare_engine, prepare_output_file, DepthSet},
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
    let (out_path, mut out_file) = prepare_output_file(input, output, args.resume)?;
    let (depths, mut engine)    = prepare_engine(args.base_depth).await?;
    let (_, _, stats)           = initialize_resume(
        input,
        out_path.parent().unwrap().to_str().unwrap(),
        args.resume,
    )?;

    let t1 = Instant::now();
    let mut ctx  = CandidateContext::new(&mut engine, None);
    let pool     = ctx.collect_candidates(Chess::default(), iterate_games(input)?, &depths).await?;
    info!("fase‑1 concluída → {} candidatos em {:.2?}", pool.len(), t1.elapsed());

    let mut total = 0u64;
    for (cand, hdrs) in pool {
        if let Some(seq) = create_puzzle_tree(
            &mut engine,
            &cand.board_post_blunder,
            cand.solver_color,
            cand.pre_cp,
            &depths,
        )
        .await?
        {
            if let Ok(pgn) = process_puzzle(&cand, &seq, &hdrs) {
                if export_puzzle(&pgn, &mut out_file).is_ok() {
                    total += 1;
                }
            }
        }
    }

    if let Err(e) = update_resume_data(
        input,
        0,
        &stats,
        out_path.parent().unwrap().to_str().unwrap(),
    ) {
        log::warn!("resume update falhou: {e}");
    }

    info!("finalizado: {total} puzzles em {:.2?}", t0.elapsed());
    Ok(GenerateResult { puzzles: total })
}
