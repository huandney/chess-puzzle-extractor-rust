// src/builder.rs
// ---------------------------------------------------------------------------
// Constrói a árvore S‑O‑S‑O… mantendo lances não‑ambíguos.
// Ajuste final: PuzzleSeq agora deriva Clone para permitir struct‑update
// em `process_puzzle`.
// ---------------------------------------------------------------------------

use anyhow::Result;
use indexmap::IndexMap;
use log::{debug, info, trace};
use shakmaty::{
    fen::Fen, CastlingSide, Chess, Color, Move, Position, Role, EnPassantMode, uci::UciMove,
};

use crate::{
    analysis::{solver_response, puzzle_is_interesting},
    candidates::PuzzleCandidate,
    config,
    engine::Engine,
    utils::{DepthSet, build_pgn_san},
};

#[derive(Debug, Clone)]
pub struct PuzzleSeq {
    pub moves:        Vec<Move>,
    pub alternatives: Vec<Vec<Move>>,
    pub final_cp:     i32,
    pub is_mate:      bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TacticalObjective { Mate, Reversal, Advantage, Equalization, Resistance, Tactical }

pub fn classify_tactic(post: i32, final_cp: i32, mate: bool) -> TacticalObjective {
    use TacticalObjective::*;
    if mate { return Mate; }
    let wa = config::WINNING_ADVANTAGE;
    let dr = config::DRAWING_RANGE;
    match (post, final_cp) {
        (p, f) if p < 0 && f >= wa         => Reversal,
        (_, f) if f >= wa                  => Advantage,
        (p, f) if p < -dr && f.abs() <= dr => Equalization,
        (p, f) if p < 0 && f < 0           => Resistance,
        _                                  => Tactical,
    }
}

pub async fn create_puzzle_tree(
    engine:       &mut Engine,
    start:        &Chess,
    solver_color: Color,
    pre_cp:       i32,
    d:            &DepthSet,
) -> Result<Option<PuzzleSeq>> {
    if !puzzle_is_interesting(engine, start, solver_color, pre_cp, d.solve).await? { return Ok(None); }

    let mut seq        = Vec::<Move>::new();
    let mut alt_lines  = Vec::<Vec<Move>>::new();
    let mut board      = start.clone();
    let mut last_cp    = pre_cp;
    let mut last_mate  = false;
    let mut solver_cnt = 0u8;

    loop {
        let sr = match solver_response(engine, &board, solver_color, pre_cp, d).await? {
            None                      => break,
            Some(r) if  r.ambiguous   => break,
            Some(r)                   => r,
        };

        seq.push(sr.solution_move.clone());
        solver_cnt += 1;
        last_cp   = sr.post_cp;
        last_mate = Engine::is_mate(&sr.score);

        if config::MAX_ALTERNATIVE_LINES > 0 {
            let keep: Vec<_> = sr.alternative_moves
                .iter()
                .take(config::MAX_ALTERNATIVE_LINES as usize)
                .cloned()
                .collect();
            if !keep.is_empty() { alt_lines.push(keep); }
        }

        board.play_unchecked(&sr.solution_move);

        let Some(bm) = engine.best_move(&board, d.solve).await? else { break };
        let reply = bm.r#move.to_move(&board)?;
        seq.push(reply.clone());
        board.play_unchecked(&reply);
    }

    if solver_cnt < config::SOLVER_MIN_MOVES { return Ok(None); }
    if seq.len() % 2 == 0 { seq.pop(); }

    Ok(Some(PuzzleSeq {
        moves:        seq,
        alternatives: alt_lines,
        final_cp:     last_cp,
        is_mate:      last_mate,
    }))
}

// ---------------------------------------------------------------------------
// Fase do jogo
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GamePhase { Opening, Middlegame, Endgame }

pub fn classify_phase(board: &Chess, plies: usize) -> GamePhase {
    const W: [i32;5] = [0,1,1,2,4];
    let mut phase = 2 * (W[0]*8 + W[1]*2 + W[2]*2 + W[3]*2 + W[4]);
    let dec = |r: Role, w: i32, v: &mut i32| *v -= w * (board.our(r).0.count_ones() + board.their(r).0.count_ones()) as i32;
    dec(Role::Pawn  , W[0], &mut phase);
    dec(Role::Knight, W[1], &mut phase);
    dec(Role::Bishop, W[2], &mut phase);
    dec(Role::Rook  , W[3], &mut phase);
    dec(Role::Queen , W[4], &mut phase);

    let material = phase as f32 / 196.0;
    let ply_norm = (plies as f32 / 80.0).min(1.0);

    let rights = [
        (Color::White, CastlingSide::KingSide),
        (Color::White, CastlingSide::QueenSide),
        (Color::Black, CastlingSide::KingSide),
        (Color::Black, CastlingSide::QueenSide),
    ].iter().filter(|&&(c,s)| board.castles().has(c,s)).count() as f32 / 4.0;

    let v = (material*2.0 + ply_norm + rights) / 4.0;
    if v >= 0.80 { GamePhase::Opening }
    else if v <= 0.20 { GamePhase::Endgame }
    else { GamePhase::Middlegame }
}

// ---------------------------------------------------------------------------
// Exporta PGN
// ---------------------------------------------------------------------------
/// Monta headers finais e delega ao `build_pgn_san`.
pub fn process_puzzle(
    cand:    &PuzzleCandidate,
    seq:     &PuzzleSeq,
    headers: &[(String, String)],        // << novo parâmetro
) -> Result<String> {
    let phase  = classify_phase(&cand.board_post_blunder, cand.move_number as usize);
    let tactic = classify_tactic(cand.post_cp, seq.final_cp, seq.is_mate);

    let mut hdr: IndexMap<String, String> =
        headers.iter().cloned().collect();                // originais

    hdr.insert("Phase".into(),    format!("{:?}", phase));
    hdr.insert("Tactical".into(), format!("{:?}", tactic));
    hdr.insert("SetUp".into(),    "1".into());
    hdr.insert(
        "FEN".into(),
        Fen::from_position(cand.board_pre_blunder.clone(), EnPassantMode::Legal).to_string(),
    );

    let mut moves = Vec::with_capacity(seq.moves.len() + 1);
    moves.push(cand.blunder_move.clone());
    moves.extend(seq.moves.iter().cloned());

    build_pgn_san(&hdr, &PuzzleSeq { moves, ..seq.clone() })
}
