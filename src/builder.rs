// src/builder.rs
// ---------------------------------------------------------------------------
// Constrói a árvore S‑O‑S‑O… usando o novo Engine.
// Corrigidos:
// • conversão UciMove → Move antes de push/play
// • não mover `sr` (usa `.iter()`)
// • eliminação de Castles::bits()
// • headers Vec<(String,String)> → IndexMap
// ---------------------------------------------------------------------------

use anyhow::Result;
use indexmap::IndexMap;
use shakmaty::{
    fen::Fen, CastlingSide, Chess, Color, Move, Position, Role, EnPassantMode,
};

use crate::{
    analysis::{solver_response, puzzle_is_interesting, SolverResponse},
    candidates::PuzzleCandidate,
    config,
    engine::Engine,
    utils::{DepthSet, build_pgn_san},
};

/// Sequência de puzzle: contém a linha principal de lances e as variantes alternativas
#[derive(Debug)]
pub struct PuzzleSeq {
    pub moves:        Vec<Move>,         // Sequência principal de lances [S,O,S,O,...]
    pub alternatives: Vec<Vec<Move>>,    // Linhas alternativas (sub-variantes)
    pub final_cp:     i32,               // Avaliação final em centipawns
    pub is_mate:      bool,              // Indica se termina em mate
}

/// Classificação do objetivo tático do puzzle
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TacticalObjective { Mate, Reversal, Advantage, Equalization, Resistance, Tactical }

/// Classifica o puzzle tático com base nas avaliações inicial e final
pub fn classify_tactic(post: i32, final_cp: i32, mate: bool) -> TacticalObjective {
    use TacticalObjective::*;
    // Mate tem prioridade absoluta
    if mate { return Mate; }

    let wa = config::WINNING_ADVANTAGE;
    let dr = config::DRAWING_RANGE;

    // Classificação baseada na evolução da avaliação
    match (post, final_cp) {
        (p, f) if p < 0 && f >= wa         => Reversal,     // De desvantagem para vantagem decisiva
        (_, f) if f >= wa                  => Advantage,    // Mantém vantagem decisiva
        (p, f) if p < -dr && f.abs() <= dr => Equalization, // De desvantagem para igualdade
        (p, f) if p < 0 && f < 0           => Resistance,   // Melhorou mas ainda em desvantagem
        _                                  => Tactical,     // Qualquer outro cenário
    }
}

/// Gera a árvore completa de lances do puzzle (solver-oponente-solver...)
pub async fn create_puzzle_tree(
    engine:       &mut Engine,
    start:        &Chess,
    solver_color: Color,
    pre_cp:       i32,
    d:            &DepthSet,
) -> Result<Option<PuzzleSeq>> {
    // Verifica se a posição é interessante antes de prosseguir
    if !puzzle_is_interesting(engine, start, solver_color, pre_cp, d.solve).await? { return Ok(None); }

    // Define o enum para saída do loop
    enum Exit { Abort(SolverResponse), Finish(SolverResponse) }

    let mut seq  = Vec::<Move>::new();       // Sequência principal de lances
    let mut alts = Vec::<Vec<Move>>::new();  // Variantes alternativas
    let mut board= start.clone();            // Tabuleiro atual
    let mut nsol = 0u8;                      // Contador de lances do solver

    // Loop principal para construir a sequência solver-oponente
    let exit = loop {
        // Obtém resposta do solver para a posição atual
        let sr = match solver_response(engine, &board, solver_color, pre_cp, d).await? {
            Some(r) if !r.ambiguous => r,                     // Resposta não ambígua: continua
            Some(r)                 => break Exit::Abort(r),  // Resposta ambígua: aborta
            None                    => return Ok(None),       // Sem resposta: descarta puzzle
        };

        // Adiciona lance do solver à sequência principal
        seq.push(sr.solution_move.clone());
        nsol += 1;

        // Coleta variantes alternativas se configurado
        if config::MAX_ALTERNATIVE_LINES > 0 {
            let keep: Vec<_> = sr.alternative_moves
                .iter()
                .take(config::MAX_ALTERNATIVE_LINES as usize)
                .cloned()
                .collect();
            if !keep.is_empty() { alts.push(keep); }
        }

        // Aplica o lance do solver no tabuleiro
        board.play_unchecked(&sr.solution_move);

        // Obtém melhor resposta do oponente
        if let Some(bm) = engine.best_move(&board, d.solve).await? {
            // Converte UciMove para Move antes de aplicar
            let reply = bm.r#move.to_move(&board)?;
            seq.push(reply.clone());
            board.play_unchecked(&reply);
        } else {
            // Sem resposta do oponente: fim natural da sequência
            break Exit::Finish(sr);
        }
    };

    // Processa resultado com base no tipo de saída
    match exit {
        Exit::Abort(_) => Ok(None),  // Abortado devido a ambiguidade
        Exit::Finish(sr) => {
            // Remove último lance se for par (termina com lance do solver)
            if seq.len() % 2 == 0 { seq.pop(); }

            // Verifica número mínimo de lances do solver
            if nsol < config::SOLVER_MIN_MOVES { return Ok(None); }

            // Retorna sequência completa
            Ok(Some(PuzzleSeq {
                moves:        seq,
                alternatives: alts,
                final_cp:     sr.post_cp,
                is_mate:      Engine::is_mate(&sr.score),
            }))
        }
    }
}

// ---------------------------------------------------------------------------
// Fase do jogo
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GamePhase { Opening, Middlegame, Endgame }

/// Classifica a fase do jogo com base no material, progresso e direitos de roque
pub fn classify_phase(board: &Chess, plies: usize) -> GamePhase {
    // Pesos das peças para cálculo de material [P,N,B,R,Q]
    const W: [i32;5] = [0,1,1,2,4];

    // Valor máximo teórico (2 × soma de todas as peças)
    let mut phase = 2 * (W[0]*8 + W[1]*2 + W[2]*2 + W[3]*2 + W[4]);

    // Função para decrementar fase baseado nas peças presentes
    let dec = |r: Role, w: i32, v: &mut i32| *v -= w * (board.our(r).0.count_ones() + board.their(r).0.count_ones()) as i32;

    // Subtrai valor para cada peça presente no tabuleiro
    dec(Role::Pawn  , W[0], &mut phase);
    dec(Role::Knight, W[1], &mut phase);
    dec(Role::Bishop, W[2], &mut phase);
    dec(Role::Rook  , W[3], &mut phase);
    dec(Role::Queen , W[4], &mut phase);

    // Normaliza valor de material para [0,1]
    let material = phase as f32 / 196.0;

    // Normaliza progresso de lances para [0,1]
    let ply_norm = (plies as f32 / 80.0).min(1.0);

    // Calcula proporção de direitos de roque ainda disponíveis
    let rights = [
        (Color::White, CastlingSide::KingSide),
        (Color::White, CastlingSide::QueenSide),
        (Color::Black, CastlingSide::KingSide),
        (Color::Black, CastlingSide::QueenSide),
    ].iter().filter(|&&(c,s)| board.castles().has(c,s)).count() as f32 / 4.0;

    // Combinação ponderada dos fatores (material tem peso duplo)
    let v = (material*2.0 + ply_norm + rights) / 4.0;

    // Classificação final baseada em thresholds
    if v >= 0.80 { GamePhase::Opening }
    else if v <= 0.20 { GamePhase::Endgame }
    else { GamePhase::Middlegame }
}

// ---------------------------------------------------------------------------
// Exporta para PGN
// ---------------------------------------------------------------------------
/// Processa o candidato e sequência para gerar PGN final do puzzle
pub fn process_puzzle(candidate: &PuzzleCandidate, seq: &PuzzleSeq) -> Result<String> {
    // Classifica a fase do jogo e o objetivo tático
    let phase  = classify_phase(&candidate.board_post_blunder, candidate.move_number as usize);
    let tactic = classify_tactic(candidate.post_cp, seq.final_cp, seq.is_mate);

    // Converte headers originais para IndexMap e adiciona novos headers
    let mut hdr: IndexMap<String,String> = candidate.original_headers.iter().cloned().collect();
    hdr.insert("Phase".into(),    format!("{:?}", phase));
    hdr.insert("Tactical".into(), format!("{:?}", tactic));
    hdr.insert("SetUp".into(),    "1".into());
    hdr.insert("FEN".into(),      Fen::from_position(candidate.board_pre_blunder.clone(), EnPassantMode::Legal).to_string());

    // Cria sequência completa começando com o blunder
    let mut moves = Vec::with_capacity(seq.moves.len() + 1);
    moves.push(candidate.blunder_move.clone());  // Primeiro lance: o blunder
    moves.extend(seq.moves.iter().cloned());     // Seguido pela sequência solver-oponente

    // Gera a sequência final completa
    let full = PuzzleSeq { moves, alternatives: seq.alternatives.clone(), final_cp: seq.final_cp, is_mate: seq.is_mate };

    // Constrói a string PGN final
    build_pgn_san(&hdr, &full)
}
