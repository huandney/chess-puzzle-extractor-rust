// src/candidates.rs
// ---------------------------------------------------------------------------
// Filtra blunders e devolve PuzzleCandidate usando o novo Engine.
// Corrigidos: acesso a score, dest() inexistente, imports obsoletos.
// ---------------------------------------------------------------------------

use anyhow::{anyhow, Result};
use log::info;
use shakmaty::{
    fen::Fen, Chess, Color, Move, Position, Square, EnPassantMode,
};

use crate::{
    config,
    engine::Engine,
    utils::DepthSet,
    visual::CustomProgressBar,
};

// ---------------------------------------------------------------------------
// Contexto
// ---------------------------------------------------------------------------
/// Contexto para avaliação de candidatos a puzzle, contendo estado do tabuleiro
/// antes e depois do blunder, movimentos e avaliações
pub struct CandidateContext<'a> {
    pub board_pre_blunder: Chess,                    // Posição antes do blunder
    pub board_post_blunder: Chess,                   // Posição depois do blunder
    pub blunder_move: Move,                          // Movimento que causou o blunder
    pub prev_cp: i32,                                // Avaliação em centipawns antes do blunder
    pub post_cp: i32,                                // Avaliação em centipawns depois do blunder
    pub scan_depth: u8,                              // Profundidade de análise inicial
    pub engine: &'a mut Engine,                      // Referência para o motor de xadrez
    pub progress_bar: Option<&'a CustomProgressBar>, // Barra de progresso opcional
    pub move_number: u32,                            // Número do lance na partida
}

impl<'a> CandidateContext<'a> {
    /// Constrói o contexto para um potencial blunder, analisando a posição
    /// antes e depois do lance jogado e verificando se a queda de avaliação é significativa
    pub async fn for_blunder(
        engine:       &'a mut Engine,
        board:        &Chess,
        actual_move:  &Move,
        d:            &DepthSet,
        progress_bar: Option<&'a CustomProgressBar>,
        move_number:  u32,
    ) -> Result<Self> {
        // Registra posição inicial antes do lance
        info!("blunder #{move_number} pré‑lance {}", Fen::from_position(board.clone(), EnPassantMode::Legal));

        // Analisa posição antes do lance
        let a_prev = engine.analyze(board, d.scan, 1).await?
            .into_iter().next().ok_or_else(|| anyhow!("sem análise pré"))?;
        let prev_cp = Engine::to_cp(a_prev.score.as_ref().unwrap());

        // Aplica o lance jogado e cria nova posição
        let mut board_post = board.clone();
        board_post.play_unchecked(actual_move);

        // Analisa posição após o lance
        let a_post = engine.analyze(&board_post, d.scan, 1).await?
            .into_iter().next().ok_or_else(|| anyhow!("sem análise pós"))?;
        let post_cp = Engine::to_cp(a_post.score.as_ref().unwrap());

        // Verifica se a queda de avaliação é suficiente para ser considerado blunder
        if prev_cp - post_cp < config::BLUNDER_THRESHOLD {
            return Err(anyhow!("queda insuficiente"));
        }

        // Retorna o contexto completo
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
/// Candidato a puzzle, contendo todas as informações necessárias para
/// gerar o puzzle completo após verificação de critérios de qualidade
#[derive(Debug, Clone)]
pub struct PuzzleCandidate {
    pub board_pre_blunder: Chess,                // Posição antes do blunder
    pub board_post_blunder: Chess,               // Posição depois do blunder
    pub adjusted_board: Chess,                   // Posição ajustada (após pular lances forçados)
    pub blunder_move: Move,                      // Lance que causou o blunder
    pub forced_sequence: Vec<Move>,              // Sequência de lances forçados (se houver)
    pub solver_color: Color,                     // Cor do jogador que resolverá o puzzle
    pub pre_cp: i32,                             // Avaliação antes do blunder
    pub post_cp: i32,                            // Avaliação depois do blunder
    pub original_headers: Vec<(String, String)>, // Headers PGN originais da partida
    pub move_number: u32,                        // Número do lance na partida
}

// ---------------------------------------------------------------------------
// Helpers simples
// ---------------------------------------------------------------------------
/// Extrai a casa de destino de um movimento (se aplicável)
fn dest_sq(m: &Move) -> Option<Square> {
    match m {
        Move::Normal { to, .. }
        | Move::EnPassant { to, .. }
        | Move::Put { to, .. } => Some(*to),
        _ => None,
    }
}

/// Detecta queda de avaliação e retorna a cor do jogador que resolverá o puzzle
pub fn detect_eval_drop(board: &Chess, prev: i32, post: i32) -> (bool, Option<Color>) {
    let diff = prev - post;
    // Se for a vez das pretas e houve queda significativa, o solver será as brancas
    if board.turn() == Color::Black && diff >= config::BLUNDER_THRESHOLD {
        return (true, Some(Color::White));
    }
    // Se for a vez das brancas e houve queda significativa, o solver será as pretas
    if board.turn() == Color::White && diff <= -config::BLUNDER_THRESHOLD {
        return (true, Some(Color::Black));
    }
    (false, None)
}

/// Verifica se há apenas um lance forçado e retorna o tabuleiro ajustado
/// e uma flag indicando se a posição é válida para puzzle
pub fn skip_forced_move(board: &Chess, solver: Color) -> (Chess, Vec<Move>, bool) {
    // Se não for o turno do solver, a posição é válida
    if board.turn() != solver { return (board.clone(), Vec::new(), true); }

    // Posição é válida apenas se houver mais de um lance legal
    (board.clone(), Vec::new(), board.legal_moves().len() > 1)
}

// ---------------------------------------------------------------------------
// Hanging piece detection
// ---------------------------------------------------------------------------
/// Verifica se há uma peça "pendurada" que pode ser capturada facilmente
async fn is_hanging(
    board: &Chess,
    engine: &mut Engine,
    target: Square,
    depth: u8,
) -> Result<bool> {
    // Analisa as duas melhores opções
    let infos = engine.analyze(board, depth, 2).await?;
    if infos.len() < 2 { return Ok(false); }

    // Verifica se o melhor lance é uma captura na casa alvo
    let cap_good = infos[0].pv.first()
        .and_then(dest_sq)
        .map_or(false, |sq| sq == target);

    if !cap_good { return Ok(false); }

    // Verifica se a diferença entre o melhor lance e o segundo
    // é significativa (indicando vantagem material clara)
    let diff = Engine::key_diff(
        infos[0].score.as_ref().unwrap(),
        infos[1].score.as_ref().unwrap(),
    );

    Ok(diff >= config::HANGING_THRESHOLD as i64)
}

// ---------------------------------------------------------------------------
// Captures sequence
// ---------------------------------------------------------------------------
/// Verifica se há uma sequência de capturas consecutivas (tática forçada)
async fn sequential_caps(board: &Chess, engine: &mut Engine, depth: u8, max: u32) -> Result<bool> {
    let mut b = board.clone();
    let mut n = 0;

    // Itera até encontrar um lance que não seja captura ou atingir o máximo
    while n < max && !b.is_game_over() {
        // Obtém o melhor lance segundo o motor
        let mv = engine.analyze(&b, depth, 1).await?
            .get(0).and_then(|i| i.pv.first()).cloned();
        let Some(m) = mv else { break; };

        // Verifica se o lance é uma captura
        let is_cap = match m {
            Move::Normal { to, .. } => b.board().piece_at(to).is_some(),
            Move::EnPassant { .. }  => true,
            _                       => false,
        };
        if !is_cap { return Ok(false); }

        // Aplica o lance e continua
        b = b.play(&m)?;
        n += 1;
    }

    // É uma sequência relevante se tiver pelo menos 2 capturas
    Ok(n >= 2)
}

// ---------------------------------------------------------------------------
// Pipeline principal
// ---------------------------------------------------------------------------
/// Analisa a posição e retorna um candidato a puzzle se atender aos critérios
pub async fn find_candidate(ctx: CandidateContext<'_>) -> Result<Option<PuzzleCandidate>> {
    // 1. Verifica se houve queda de avaliação suficiente
    let (blunder, solver_color) = detect_eval_drop(&ctx.board_pre_blunder, ctx.prev_cp, ctx.post_cp);
    if !blunder { return Ok(None); }
    let solver = solver_color.unwrap();

    // 2. Verifica se a posição não tem lance forçado
    let (adjusted, forced, valid) = skip_forced_move(&ctx.board_post_blunder, solver);
    if !valid { return Ok(None); }

    // 3. Verifica se não é uma simples peça pendurada
    if let Some(tgt) = dest_sq(&ctx.blunder_move) {
        if is_hanging(&ctx.board_post_blunder, ctx.engine, tgt, ctx.scan_depth).await? {
            return Ok(None);
        }
    }

    // 4. Verifica se não é uma sequência simples de capturas
    if sequential_caps(&ctx.board_post_blunder, ctx.engine, ctx.scan_depth, 5).await? {
        return Ok(None);
    }

    // 5. Retorna o candidato a puzzle se passar em todos os filtros
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
