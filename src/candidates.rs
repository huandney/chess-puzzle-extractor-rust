// src/candidates.rs
// ---------------------------------------------------------------------------
// Blunder‑scan minimalista: 1 avaliação por posição e apenas o filtro
// “solver tem pelo menos 2 jogadas”.  Todo o pipeline é realizado em
// CandidateContext::find_candidate, que devolve Option<PuzzleCandidate>.
// ---------------------------------------------------------------------------

use anyhow::{anyhow, Result};
use shakmaty::{Chess, Color, Move, Position};

use crate::{config, engine::Engine, utils::DepthSet, visual::CustomProgressBar};

// ---------------------------------------------------------------------------
// Estruturas públicas
// ---------------------------------------------------------------------------
pub struct CandidateContext<'a> {
    pub engine:       &'a mut Engine,
    pub progress_bar: Option<&'a CustomProgressBar>,
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

// ---------------------------------------------------------------------------
// Pipeline completo – única análise por posição
// ---------------------------------------------------------------------------
impl<'a> CandidateContext<'a> {
    /// Analisa board + mv. Requer prev_cp já disponível para economizar
    /// CPU.  Retorna Ok(Some(candidate)) se passar em todos filtros leves.
    pub async fn find_candidate(
        &mut self,
        board_pre : &Chess,
        mv        : &Move,
        prev_cp   : i32,
        depth_set : &DepthSet,
        move_no   : u32,
    ) -> Result<Option<PuzzleCandidate>> {
        // posição pós‑blunder
        let mut board_post = board_pre.clone();
        board_post.play_unchecked(mv);

        // avaliação pós (única consulta ao motor)
        let post_std = self.engine.analyze(&board_post, depth_set.scan, 1).await?
            .get(0).and_then(|i| i.score.clone())
            .ok_or_else(|| anyhow!("análise pós‑blunder ausente"))?;

        // converte para centipawns o post-blunder
        let post_cp = Engine::to_cp(&post_std);
        // diferença absoluta em centipawns
        let diff = (post_cp - prev_cp).abs() as i64;
        // fail-fast: descarta quedas menores que o limiar
        if diff < config::BLUNDER_THRESHOLD as i64 {
            return Ok(None);
        }
        // decide quem será o solver
        let solver = if post_cp > prev_cp { Color::White } else { Color::Black };

        // filtro trivialidade: solver deve ter ≥2 escolhas
        if board_post.legal_moves().len() <= 1 { return Ok(None); }

        Ok(Some(PuzzleCandidate {
            board_pre_blunder : board_pre.clone(),
            board_post_blunder: board_post,
            blunder_move      : mv.clone(),
            solver_color      : solver,
            pre_cp            : prev_cp,
            post_cp           : post_cp,
            move_number       : move_no,
        }))
    }
}
