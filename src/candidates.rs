// src/candidates.rs
// ---------------------------------------------------------------------------
// Varredura de blunders: 1 chamada ao engine por lance. Fila de candidatos.
// ---------------------------------------------------------------------------

use anyhow::Result;
use shakmaty::{Chess, Color, Move, Position};
use crate::{
    config,
    engine::Engine,
    utils::{DepthSet, MoveRecord},
    visual::CustomProgressBar,
};

pub struct CandidateContext<'a> {
    engine:       &'a mut Engine,
    progress_bar: Option<&'a CustomProgressBar>,
}

#[derive(Debug, Clone)]
pub struct PuzzleCandidate {
    pub board_pre_blunder : Chess,
    pub board_post_blunder: Chess,
    pub blunder_move      : Move,
    pub solver_color      : Color,
    pub pre_cp            : i32,
    pub post_cp           : i32,
    pub move_number       : u32,
}

impl<'a> CandidateContext<'a> {
    #[inline]
    pub fn new(
        engine:       &'a mut Engine,
        progress_bar: Option<&'a CustomProgressBar>,
    ) -> Self {
        Self { engine, progress_bar }
    }

    pub async fn collect_candidates<I>(
        &mut self,
        mut board: Chess,
        games: I,
        depths: &DepthSet,
    ) -> Result<Vec<(PuzzleCandidate, Vec<(String, String)>)>>
    where
        I: IntoIterator<Item = MoveRecord>,
    {
        let init = self.engine.analyze(&board, depths.scan, 1).await?[0]
            .score.as_ref().unwrap().clone();
        let mut prev_cp = Engine::to_cp(&init);
        let mut pool = Vec::new();

        for rec in games {
            let (next_cp, maybe_cand) = self
                .find_candidate(&board, &rec.mv, prev_cp, depths, rec.move_idx)
                .await?;
            if let Some(cand) = maybe_cand {
                pool.push((cand, rec.headers));
            }
            board.play_unchecked(&rec.mv);
            prev_cp = next_cp;
        }

        Ok(pool)
    }

    async fn find_candidate(
        &mut self,
        board_pre: &Chess,
        mv:        &Move,
        prev_cp:   i32,
        depths:    &DepthSet,
        move_no:   u32,
    ) -> Result<(i32, Option<PuzzleCandidate>)> {
        let mut post = board_pre.clone();
        post.play_unchecked(mv);

        // falha rápido: sem jogadas → posição terminal
        if post.legal_moves().len() < 1 {
            return Ok((prev_cp, None));
        }

        let std = self.engine.analyze(&post, depths.scan, 1).await?[0]
            .score.as_ref().unwrap().clone();
        let post_cp = Engine::to_cp(&std);
        let diff = (post_cp - prev_cp).abs() as i64;
        if diff < config::BLUNDER_THRESHOLD as i64 {
            return Ok((post_cp, None));
        }

        let solver = if post_cp > prev_cp { Color::White } else { Color::Black };
        if post.legal_moves().len() <= 1 {
            return Ok((post_cp, None));
        }

        Ok((
            post_cp,
            Some(PuzzleCandidate {
                board_pre_blunder : board_pre.clone(),
                board_post_blunder: post,
                blunder_move      : mv.clone(),
                solver_color      : solver,
                pre_cp            : prev_cp,
                post_cp,
                move_number       : move_no,
            }),
        ))
    }
}
