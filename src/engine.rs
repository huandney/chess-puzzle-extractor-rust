// src/engine.rs
// ---------------------------------------------------------------------------
//  • Syzygy DTZ (≤7 peças) com escolha adaptativa (win → menor DTZ, loss → maior)
//  • env::split_paths para SYZYGY_PATHS (funciona em Windows/Unix)
//  • AnalysisOrigin enum em AnalysisInfo
//  • Helpers key / key_diff / is_mate / to_cp
// ---------------------------------------------------------------------------

use std::{
    borrow::Cow,
    collections::HashMap,
    env,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{anyhow, Context, Result};
use futures_util::future::ready;
use log::trace;
use shakmaty::{
    fen::Fen, CastlingMode, Chess, Color, EnPassantMode, Move as ShakMove, Position, uci::UciMove,
};
use tokio::{
    io::BufReader,
    process::{Child, ChildStdin, ChildStdout, Command},
    time::timeout,
};
use ruci::{
    engine::{Info, NormalBestMove, Score as RuciScore, ScoreStandardized},
    gui::{traits::Message as UciMessage, Go, IsReady, Position as UciPosition, Quit, SetOption},
    Engine as RuciEngine,
};
use shakmaty_syzygy::{Tablebase, Wdl, MaybeRounded, Dtz};

use crate::{config, utils::DepthSet};

// ---------------------------------------------------------------------------
// Constantes
// ---------------------------------------------------------------------------
const DEFAULT_TIMEOUT_MS: u64 = 10_000;
const ANALYSIS_FACTOR:    u64 = 2;
const MATE_KEY_OFFSET:    i64 = 2_000_000;

// ---------------------------------------------------------------------------
// Tipos públicos
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, Copy)]
pub enum AnalysisOrigin { Engine, Syzygy }

#[derive(Clone, Debug)]
pub struct AnalysisInfo {
    pub score:    Option<ScoreStandardized>,
    pub depth:    Option<u8>,
    pub seldepth: Option<u8>,
    pub nodes:    Option<u64>,
    pub pv:       Vec<ShakMove>,
    pub origin:   AnalysisOrigin,
}

pub struct Engine {
    inner:       RuciEngine<BufReader<ChildStdout>, ChildStdin>,
    child:       Child,
    timeout_ms:  u64,
    current_mpv: u32,
    tb:          Option<Tablebase<Chess>>,
    start:       Instant,
}

// ---------------------------------------------------------------------------
// Implementação
// ---------------------------------------------------------------------------
impl Engine {
    // ---------- criação ----------
    pub async fn new(path: &str) -> Result<Self> {
        let mut child = Command::new(path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .with_context(|| format!("Falha ao executar '{path}'"))?;

        let stdout = child.stdout.take().ok_or_else(|| anyhow!("stdout indisponível"))?;
        let stdin  = child.stdin .take().ok_or_else(|| anyhow!("stdin indisponível"))?;

        let mut inner = RuciEngine { engine: BufReader::new(stdout), gui: stdin, strict: false };
        timeout(Duration::from_millis(DEFAULT_TIMEOUT_MS), inner.use_uci_async(|_| ready(()))).await??;

        for (k, v) in &[("Threads", config::THREADS), ("Hash", config::HASH_MB)] {
            inner.send_async(SetOption { name: Cow::Borrowed(k), value: Some(Cow::Owned(v.to_string())) }).await?;
        }
        inner.send_async(IsReady).await?;
        timeout(Duration::from_millis(DEFAULT_TIMEOUT_MS), inner.is_ready_async()).await??;

        Ok(Self {
            inner,
            child,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            current_mpv: 1,
            tb: load_syzygy(),
            start: Instant::now(),
        })
    }

    // ---------- helpers públicos ----------
    #[inline] pub fn key(s: &ScoreStandardized) -> i64 {
        match s.score() {
            RuciScore::Centipawns(cp)      => cp as i64,
            RuciScore::MateIn(m) if m >= 0 => MATE_KEY_OFFSET - m as i64,
            RuciScore::MateIn(m)           => -MATE_KEY_OFFSET - m as i64,
        }
    }
    #[inline] pub fn key_diff(a: &ScoreStandardized, b: &ScoreStandardized) -> i64 { (Self::key(a) - Self::key(b)).abs() }
    #[inline] pub fn is_mate(s: &ScoreStandardized) -> bool { matches!(s.score(), RuciScore::MateIn(_)) }
    #[inline] pub fn to_cp(std: &ScoreStandardized) -> i32 {
        match std.score() {
            RuciScore::Centipawns(cp)      => cp as i32,
            RuciScore::MateIn(p) if p >= 0 => 100_000 - p as i32,
            RuciScore::MateIn(p)           => -100_000 - p as i32,
        }
    }

    // ---------- internos ----------
    async fn send<C>(&mut self, cmd: C) -> Result<()>
    where C: UciMessage + std::fmt::Debug + Send + 'static
    {
        trace!("› {:?}", cmd);
        timeout(Duration::from_millis(self.timeout_ms), self.inner.send_async(cmd)).await??;
        Ok(())
    }

    async fn ready(&mut self) -> Result<()> {
        self.send(IsReady).await?;
        timeout(Duration::from_millis(self.timeout_ms), self.inner.is_ready_async()).await??;
        Ok(())
    }

    async fn set_position(&mut self, board: &Chess) -> Result<()> {
        let fen = Fen::from_position(board.clone(), EnPassantMode::Legal);
        self.send(UciPosition::Fen { fen: Cow::Owned(fen), moves: Cow::Owned(Vec::new()) }).await?;
        self.ready().await
    }

    async fn ensure_mpv(&mut self, mpv: u32) -> Result<()> {
        if mpv == self.current_mpv { return Ok(()); }
        self.send(SetOption { name: Cow::Borrowed("MultiPV"), value: Some(Cow::Owned(mpv.to_string())) }).await?;
        self.ready().await?;
        self.current_mpv = mpv;
        Ok(())
    }

    // ---------- análise ----------
    pub async fn analyze(&mut self, board: &Chess, depth: u8, mpv: u32) -> Result<Vec<AnalysisInfo>> {
        if let Some(ref tb) = self.tb {
            if board.board().occupied().into_iter().count() <= 7 {
                return Ok(vec![probe_tablebase(board, tb)?]);
            }
        }

        self.set_position(board).await?;
        self.ensure_mpv(mpv).await?;

        let go = Go { depth: Some(depth as usize), ..Default::default() };
        let map: Arc<Mutex<HashMap<u32, AnalysisInfo>>> = Arc::new(Mutex::new(HashMap::new()));
        let cb = map.clone();

        let limit = Duration::from_millis(self.timeout_ms * ANALYSIS_FACTOR * depth as u64);
        timeout(limit, self.inner.go_async(&go, move |info: Info| {
            if let (Some(id), Some(_)) = (info.multi_pv, info.score.as_ref()) {
                if !info.pv.is_empty() {
                    cb.lock().unwrap().insert(id as u32, convert_info(&info, board.turn(), board));
                }
            }
            ready(())
        })).await??;

        let mut lines: Vec<_> = Arc::try_unwrap(map).unwrap().into_inner().unwrap().into_values().collect();
        let sign = if board.turn() == Color::White { -1 } else { 1 };
        lines.sort_by_key(|i| i.score.as_ref().map_or(i64::MIN, |s| sign * Self::key(s)));
        Ok(lines)
    }

    // ---------- wrappers FEN ----------
    pub async fn analyze_fen(&mut self, fen: &str, depth: u8, mpv: u32) -> Result<Vec<AnalysisInfo>> {
        let pos: Chess = fen.parse::<Fen>()?.into_position(CastlingMode::Standard)?;
        self.analyze(&pos, depth, mpv).await
    }

    pub async fn best_move(&mut self, board: &Chess, depth: u8) -> Result<Option<NormalBestMove>> {
        let mv_opt = self.analyze(board, depth, 1).await?
            .pop()
            .and_then(|i| i.pv.first().cloned());

        Ok(mv_opt.map(|m| NormalBestMove {
            r#move:  UciMove::from_move(&m, CastlingMode::Standard),
            ponder: None,
        }))
    }

    pub async fn best_move_fen(&mut self, fen: &str, depth: u8) -> Result<Option<NormalBestMove>> {
        let pos: Chess = fen.parse::<Fen>()?.into_position(CastlingMode::Standard)?;
        self.best_move(&pos, depth).await
    }

    // ---------- utilitários ----------
    pub async fn scan_position(&mut self, b: &Chess, d: DepthSet) -> Result<Vec<AnalysisInfo>> {
        self.analyze(b, d.scan, 1).await
    }
    pub async fn solve_position(&mut self, b: &Chess, d: DepthSet, mpv: u32) -> Result<Vec<AnalysisInfo>> {
        self.analyze(b, d.solve, mpv).await
    }
    pub async fn quit(&mut self) -> Result<()> {
        let _ = self.send(Quit).await;
        let _ = timeout(Duration::from_millis(1_000), self.child.wait()).await;
        Ok(())
    }
}

impl Drop for Engine { fn drop(&mut self) { let _ = self.child.start_kill(); } }

// ---------------------------------------------------------------------------
// Helpers Syzygy
// ---------------------------------------------------------------------------
fn load_syzygy() -> Option<Tablebase<Chess>> {
    let paths = env::var("SYZYGY_PATHS").ok()?;
    let mut tb = Tablebase::new();
    for dir in env::split_paths(&paths) { let _ = tb.add_directory(&dir); }
    (tb.max_pieces() > 0).then_some(tb)
}

/// DTZ negativo → mate em |dtz| plies, DTZ positivo → distância até escapar do mate.
/// Critério: se WDL=Loss, escolhe maior DTZ; caso contrário, menor DTZ.
fn probe_tablebase(board: &Chess, tb: &Tablebase<Chess>) -> Result<AnalysisInfo> {
    use RuciScore::*;
    let mut best: Option<(i32, ShakMove)> = None;

    for mv in board.legal_moves() {
        let pos = board.clone().play(&mv)?;
        let wdl = tb.probe_wdl_after_zeroing(&pos)?;
        let dtz_raw: MaybeRounded<Dtz> = tb.probe_dtz(&pos)?;
        let dtz = match dtz_raw { MaybeRounded::Rounded(Dtz(v)) | MaybeRounded::Precise(Dtz(v)) => v as i32 };

        let want_min = wdl != Wdl::Loss;                     // perdendo? queremos maximizar
        let better = best.as_ref().map_or(true, |(prev, _)| if want_min { dtz < *prev } else { dtz > *prev });
        if better { best = Some((dtz, mv)); }
    }

    let (dtz, first_move) = best.ok_or_else(|| anyhow!("tablebase não gerou movimento"))?;
    let raw_score = if dtz < 0 { MateIn((-dtz) as isize) } else { Centipawns(0) };
    let score = raw_score.standardized(board.turn());

    Ok(AnalysisInfo {
        score: Some(score),
        depth: None,
        seldepth: None,
        nodes: None,
        pv: vec![first_move],
        origin: AnalysisOrigin::Syzygy,
    })
}

// ---------------------------------------------------------------------------
//  Conversão Info → AnalysisInfo
// ---------------------------------------------------------------------------
fn convert_info(src: &Info, turn: Color, board: &Chess) -> AnalysisInfo {
    let score    = src.score.as_ref().map(|s| s.kind.standardized(turn));
    let depth    = src.depth.map(|d| d.depth as u8);
    let seldepth = src.depth.and_then(|d| d.seldepth.map(|s| s as u8));
    let nodes    = src.nodes.map(|n| n as u64);
    let pv       = src.pv.iter().filter_map(|uci| uci.to_move(board).ok()).collect();
    AnalysisInfo { score, depth, seldepth, nodes, pv, origin: AnalysisOrigin::Engine }
}
