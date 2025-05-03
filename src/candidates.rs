// src/candidates.rs
// ---------------------------------------------------------------------------
// Detecta blunders avaliando variação absoluta ≥ BLUNDER_THRESHOLD.
// • `detect_blunder` devolve `Option<Color>` (solver)    ─ reduz duplicação
// • Side que resolverá é quem se beneficia da queda
// • Funções auxiliares simplificadas, sem encadeamentos else
// ---------------------------------------------------------------------------

use anyhow::{anyhow, Result};
use log::info;
use shakmaty::{fen::Fen, Chess, Color, Move, Position, Square, EnPassantMode};

use crate::{config, engine::Engine, utils::DepthSet, visual::CustomProgressBar};

// ---------------------------------------------------------------------------
// Contexto
// ---------------------------------------------------------------------------
pub struct CandidateContext<'a> {
    pub board_pre_blunder: Chess,
    pub board_post_blunder: Chess,
    pub blunder_move: Move,
    pub prev_cp: i32,
    pub post_cp: i32,
    pub scan_depth: u8,
    pub engine: &'a mut Engine,
    pub progress_bar: Option<&'a CustomProgressBar>,
    pub move_number: u32,
}

impl<'a> CandidateContext<'a> {
    pub async fn for_blunder(
        engine:       &'a mut Engine,
        board:        &Chess,
        actual_move:  &Move,
        d:            &DepthSet,
        progress_bar: Option<&'a CustomProgressBar>,
        move_number:  u32,
    ) -> Result<Self> {
        info!("blunder #{move_number} pré‑lance {}",
              Fen::from_position(board.clone(), EnPassantMode::Legal));

        let prev_cp = Engine::to_cp(
            engine.analyze(board, d.scan, 1).await?
                .into_iter().next().ok_or_else(|| anyhow!("sem análise pré"))?
                .score.as_ref().unwrap());

        let mut board_post = board.clone();
        board_post.play_unchecked(actual_move);

        let post_cp = Engine::to_cp(
            engine.analyze(&board_post, d.scan, 1).await?
                .into_iter().next().ok_or_else(|| anyhow!("sem análise pós"))?
                .score.as_ref().unwrap());

        let diff = (post_cp - prev_cp).abs();
        if diff < config::BLUNDER_THRESHOLD { return Err(anyhow!("queda insuficiente")); }

        Ok(Self {
            board_pre_blunder: board.clone(),
            board_post_blunder: board_post,
            blunder_move: actual_move.clone(),
            prev_cp,
            post_cp,
            scan_depth: d.scan,
            engine,
            progress_bar,
            move_number,
        })
    }
}

// ---------------------------------------------------------------------------
// Estrutura final
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct PuzzleCandidate {
    pub board_pre_blunder: Chess,
    pub board_post_blunder: Chess,
    pub adjusted_board: Chess,
    pub blunder_move: Move,
    pub forced_sequence: Vec<Move>,
    pub solver_color: Color,
    pub pre_cp: i32,
    pub post_cp: i32,
    pub original_headers: Vec<(String, String)>,
    pub move_number: u32,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------
fn dest_sq(m: &Move) -> Option<Square> {
    match m {
        Move::Normal { to, .. } | Move::EnPassant { to, .. } | Move::Put { to, .. } => Some(*to),
        _ => None,
    }
}

/// Retorna `Some(solver_color)` se houve blunder; caso contrário `None`.
fn detect_blunder(prev_cp: i32, post_cp: i32) -> Option<Color> {
    let diff = post_cp - prev_cp;                       // positivo: vantagem branca ↑
    (diff.abs() >= config::BLUNDER_THRESHOLD).then(|| if diff > 0 { Color::White } else { Color::Black })
}

/// Posição é válida se turno ≠ solver OU existirem ≥2 lances legais.
fn skip_forced_move(board: &Chess, solver: Color) -> (Chess, Vec<Move>, bool) {
    if board.turn() != solver { return (board.clone(), Vec::new(), true); }
    (board.clone(), Vec::new(), board.legal_moves().len() > 1)
}

// ---------------------------------------------------------------------------
// Hanging piece detection
// ---------------------------------------------------------------------------
async fn is_hanging(board: &Chess, engine: &mut Engine, target: Square, depth: u8) -> Result<bool> {
    let infos = engine.analyze(board, depth, 2).await?;
    if infos.len() < 2 { return Ok(false); }

    let best_is_cap = infos[0].pv.first().and_then(dest_sq).map_or(false, |sq| sq == target);
    if !best_is_cap { return Ok(false); }

    let diff = Engine::key_diff(
        infos[0].score.as_ref().unwrap(),
        infos[1].score.as_ref().unwrap(),
    );
    Ok(diff >= config::HANGING_THRESHOLD as i64)
}

// ---------------------------------------------------------------------------
// Sequential captures
// ---------------------------------------------------------------------------
async fn sequential_caps(board: &Chess, engine: &mut Engine, depth: u8, max: u32) -> Result<bool> {
    let mut b = board.clone();
    let mut n = 0;
    while n < max && !b.is_game_over() {
        let mv = engine.analyze(&b, depth, 1).await?
            .get(0).and_then(|i| i.pv.first()).cloned();
        let Some(m) = mv else { break };

        let capture = match m {
            Move::Normal { to, .. } => b.board().piece_at(to).is_some(),
            Move::EnPassant { .. }  => true,
            _                       => false,
        };
        if !capture { return Ok(false) }

        b = b.play(&m)?;
        n += 1;
    }
    Ok(n >= 2)
}

// ---------------------------------------------------------------------------
// Pipeline
// ---------------------------------------------------------------------------
pub async fn find_candidate(ctx: CandidateContext<'_>) -> Result<Option<PuzzleCandidate>> {
    let Some(solver) = detect_blunder(ctx.prev_cp, ctx.post_cp) else { return Ok(None) };

    let (adjusted, forced, valid) = skip_forced_move(&ctx.board_post_blunder, solver);
    if !valid { return Ok(None); }

    if let Some(tgt) = dest_sq(&ctx.blunder_move) {
        if is_hanging(&ctx.board_post_blunder, ctx.engine, tgt, ctx.scan_depth).await? { return Ok(None); }
    }

    if sequential_caps(&ctx.board_post_blunder, ctx.engine, ctx.scan_depth, 5).await? { return Ok(None); }

    Ok(Some(PuzzleCandidate {
        board_pre_blunder: ctx.board_pre_blunder.clone(),
        board_post_blunder: ctx.board_post_blunder.clone(),
        adjusted_board: adjusted,
        blunder_move: ctx.blunder_move.clone(),
        forced_sequence: forced,
        solver_color: solver,
        pre_cp: ctx.prev_cp,
        post_cp: ctx.post_cp,
        original_headers: Vec::new(),
        move_number: ctx.move_number,
    }))
}
