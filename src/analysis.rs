// src/analysis.rs
// ---------------------------------------------------------------------------
// Módulo de Análise – usa apenas helpers do novo Engine
// ---------------------------------------------------------------------------
//  * Ordenação: Engine::key()               (uma única linha)
//  * Clusterização: Engine::key_diff()
//  * Ambiguidade:  Engine::key_diff()
//  * Conversão CP: Engine::to_cp()
//  * Fail‑fast, sem funções auxiliares redundantes
// ---------------------------------------------------------------------------
use anyhow::Result;
use shakmaty::{Chess, Color, Move};
use crate::{
    config,
    engine::{AnalysisInfo, Engine},
    utils::DepthSet,
};
use ruci::engine::ScoreStandardized;

/// Resposta do solver.
#[derive(Debug, Clone)]
pub struct SolverResponse {
    pub solution_move:     Move,
    pub alternative_moves: Vec<Move>,
    pub ambiguous:         bool,
    pub score:             ScoreStandardized,
    pub post_cp:           i32,
}

/// Analisa, clusteriza e gera SolverResponse.
///
/// Realiza análise do tabuleiro com o motor, ordenando os resultados com base
/// na cor do solucionador (jogador que resolverá o puzzle). Em seguida,
/// agrupa lances equivalentes em um cluster e verifica se existe ambiguidade
/// na solução, ou seja, se existem múltiplos lances com avaliação similar.
pub async fn solver_response(
    engine:       &mut Engine,
    board:        &Chess,
    solver_color: Color,
    _pre_cp:      i32,
    depths:       &DepthSet,
) -> Result<Option<SolverResponse>> {
    // Obtém análise do motor na profundidade de solução
    let infos = engine
        .analyze(board, depths.solve, (config::MAX_ALTERNATIVE_LINES as u32) + 2)
        .await?;
    if infos.is_empty() { return Ok(None); }

    // Determina o modificador de sinal para ordenação com base na cor do solver
    // Para peças brancas, inverte a ordenação (multiplica por -1)
    let sign = if solver_color == Color::White { -1 } else { 1 };

    // Filtra e ordena os resultados da análise
    let mut ordered: Vec<&AnalysisInfo> = infos
        .iter()
        .filter(|i| i.score.is_some() && !i.pv.is_empty())
        .collect();
    ordered.sort_by_key(|i| sign * Engine::key(i.score.as_ref().unwrap()));
    if ordered.is_empty() { return Ok(None); }

    // Obtém a pontuação do melhor lance
    let base = ordered[0].score.as_ref().unwrap();

    // Define threshold de cluster diferente para mates vs. vantagem material
    let thr  = if Engine::is_mate(base) { config::MATE_ALT_THRESHOLD as i64 }
               else                     { config::ALT_THRESHOLD       as i64 };

    // Agrupa lances similares dentro do threshold definido
    // Isso captura variações equivalentes para a mesma tática
    let cluster: Vec<Move> = ordered
        .iter()
        .take_while(|i| Engine::key_diff(base, i.score.as_ref().unwrap()) <= thr)
        .filter_map(|i| i.pv.first().cloned())
        .collect();
    if cluster.is_empty() { return Ok(None); }

    // Verifica se há ambiguidade: se o próximo melhor lance fora do cluster
    // está muito próximo em avaliação do melhor lance do cluster
    let ambiguous = ordered.len() > cluster.len()
        && Engine::key_diff(
               base,
               ordered[cluster.len()].score.as_ref().unwrap(),
           ) < config::PUZZLE_UNICITY_THRESHOLD as i64;

    Ok(Some(SolverResponse {
        solution_move:     cluster[0].clone(),
        alternative_moves: cluster.into_iter().skip(1).collect(),
        ambiguous,
        score:   base.clone(),
        post_cp: Engine::to_cp(base),
    }))
}

/// Verifica se a posição permanece interessante após o lance do solver.
///
/// Uma posição é considerada interessante quando:
/// 1. A vantagem não é completamente decisiva (menor que COMPLETELY_WINNING_THRESHOLD)
/// 2. Ou se a segunda melhor opção:
///    - Está dentro da margem de empate
///    - Ou representa uma reversão de vantagem (de vantagem para desvantagem)
pub async fn puzzle_is_interesting(
    engine:       &mut Engine,
    board:        &Chess,
    _solver:      Color,
    pre_cp:       i32,
    depth:        u8,
) -> Result<bool> {
    // Se a vantagem não é decisiva, a posição já é considerada interessante
    if pre_cp.abs() < config::COMPLETELY_WINNING_THRESHOLD { return Ok(true); }

    // Analisa para verificar outras opções
    let infos = engine.analyze(board, depth, 2).await?;
    if infos.len() < 2 { return Ok(true); }

    // Avalia o segundo melhor lance
    let second_cp = Engine::to_cp(infos[1].score.as_ref().unwrap());

    // Posição é interessante se:
    // 1. O segundo melhor lance está próximo do empate
    // 2. Há uma mudança significativa de valor (reversão)
    Ok(second_cp.abs() <= config::DRAWING_RANGE
        || (pre_cp > 0 && second_cp < -config::DRAWING_RANGE)
        || (pre_cp < 0 && second_cp >  config::DRAWING_RANGE))
}
