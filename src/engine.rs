use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex,
    time::{timeout, Duration},
};
use shakmaty::{Chess, Move as ShakMove, Position, uci::UciMove, fen::Fen, CastlingMode};
use shakmaty_syzygy::{Tablebase, AmbiguousWdl};
use std::{sync::Arc, cmp::Ordering, collections::HashMap};
use anyhow::{Result, anyhow};
use crate::config::{THREADS, HASH_MB};

const ENGINE_TIMEOUT: Duration = Duration::from_secs(10);

/// Score retornado pelo engine ou tablebase
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Score { Cp(i32), Mate(i32) }

impl Ord for Score {
    fn cmp(&self, other: &Self) -> Ordering {
        use Score::*;
        match (self, other) {
            (Mate(a), Mate(b)) => b.cmp(a),
            (Mate(_), Cp(_))   => Ordering::Greater,
            (Cp(_), Mate(_))   => Ordering::Less,
            (Cp(a), Cp(b))     => a.cmp(b),
        }
    }
}

impl PartialOrd for Score {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Origem da análise
#[derive(Debug, Clone)]
enum AnalysisOrigin { Engine, Syzygy }

/// Informações de cada linha de análise
#[derive(Debug, Clone)]
pub struct AnalysisInfo {
    pub score: Score,  // Adicione 'pub' aqui
    pub depth: u8,
    pub pv: Vec<ShakMove>,
    pub origin: AnalysisOrigin,
    pub multipv: usize,
}

/// Engine UCI + tablebase incremental
pub struct Engine {
    child:           Child,
    stdin:           Arc<Mutex<ChildStdin>>,
    stdout:          Arc<Mutex<BufReader<ChildStdout>>>,
    syzygy:          Option<Tablebase<Chess>>,
    board:           Chess,
    moves:           Vec<String>,
    castling_mode:   CastlingMode,
    position_cmd:    String,
    position_synced: bool,
    current_multipv: usize,
}

impl Drop for Engine { fn drop(&mut self) { let _ = self.child.kill(); }}

impl Engine {
    /// Espera por resposta "uciok" após comando "uci" per UCI spec: engine deve enviar "uciok" após options
    async fn wait_uci(&self) -> Result<()> {
        let mut buf = String::new();
        loop {
            let line = {
                let mut r = self.stdout.lock().await;
                r.read_line(&mut buf).await?;
                buf.clone()
            };
            buf.clear();
            if line.trim() == "uciok" { break; }
        }
        Ok(())
    }

    /// Envia "isready" e espera por "readyok"; essencial após setoption e ucinewgame
    #[inline]
    async fn wait_ready(&self) -> Result<()> {
        self.cmd("isready").await?;
        let mut buf = String::new();
        loop {
            let line = {
                let mut r = self.stdout.lock().await;
                r.read_line(&mut buf).await?;
                buf.clone()
            };
            buf.clear();
            if line.trim() == "readyok" { break; }
        }
        Ok(())
    }
    /// Cria engine com tablebase opcional
    pub async fn new_with_syzygy(path: &str, tb_dirs: &[&str]) -> Result<Self> {
        let mut child = Command::new(path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()?;
        let stdin = Arc::new(Mutex::new(child.stdin.take().unwrap()));
        let stdout = Arc::new(Mutex::new(BufReader::new(child.stdout.take().unwrap())));
        let mut tb = Tablebase::<Chess>::new();
        for d in tb_dirs { tb.add_directory(d)?; }
        let engine = Engine {
            child,
            stdin,
            stdout,
            syzygy: Some(tb),
            board: Chess::default(),
            moves: Vec::new(),
            castling_mode: CastlingMode::Standard,
            position_cmd: "position startpos".into(),
            position_synced: false,
            current_multipv: 0,
        };
        engine.cmd("uci").await?;
        engine.wait_ready().await?;
        engine.cmd(&format!("setoption name Threads value {}", THREADS)).await?;
        engine.cmd(&format!("setoption name Hash value {}", HASH_MB)).await?;
        engine.wait_ready().await?;
        Ok(engine)
    }

    /// Cria engine sem tablebase
    pub async fn new(path: &str) -> Result<Self> {
        Self::new_with_syzygy(path, &[]).await
    }

    /// Reinicia jogo interno (limpa moves), espera readyok
    pub async fn new_game(&mut self) -> Result<()> {
        self.board = Chess::default();
        self.moves.clear();
        self.castling_mode = CastlingMode::Standard;
        self.position_cmd = "position startpos".into();
        self.position_synced = false;
        self.current_multipv = 0;
        self.cmd("ucinewgame").await?;
        self.wait_ready().await?;
        Ok(())
    }

    /// Garante que position_cmd foi enviado
    #[inline]
    async fn ensure_synced(&mut self) -> Result<()> {
        if !self.position_synced {
            self.cmd(&self.position_cmd).await?;
            self.position_synced = true;
        }
        Ok(())
    }

    /// Aplica lance e marca desincronizado
    pub async fn push_move(&mut self, mv: &ShakMove) -> Result<()> {
        Position::play_unchecked(&mut self.board, mv);
        let uci = UciMove::from_move(mv, self.castling_mode).to_string();
        if self.moves.is_empty() {
            self.position_cmd.push_str(" moves ");
            self.position_cmd.push_str(&uci);
        } else {
            self.position_cmd.push(' ');
            self.position_cmd.push_str(&uci);
        }
        self.moves.push(uci);
        self.position_synced = false;
        Ok(())
    }

    /// Analisa FEN sem alterar estado interno
    pub async fn analyze_fen(&mut self, fen: &str, depth: u8, multipv: usize) -> Result<Vec<AnalysisInfo>> {
        let old_board = self.board.clone();
        let old_moves = self.moves.clone();
        let old_cmd = self.position_cmd.clone();
        let old_sync = self.position_synced;
        let old_mpv = self.current_multipv;
        let fen_struct: Fen = fen.parse()?;
        self.board = fen_struct.into_position(self.castling_mode)?;
        self.moves.clear();
        self.position_cmd = format!("position fen {}", fen);
        self.position_synced = false;
        self.current_multipv = 0;
        let res = self.analyze(depth, multipv).await;
        self.board = old_board;
        self.moves = old_moves;
        self.position_cmd = old_cmd;
        self.position_synced = old_sync;
        self.current_multipv = old_mpv;
        res
    }

    /// Envia comando UCI
    #[inline]
    async fn cmd(&self, c: &str) -> Result<()> {
        let mut w = self.stdin.lock().await;
        w.write_all(c.as_bytes()).await?;
        w.write_all(b"\n").await?;
        w.flush().await?;
        Ok(())
    }

    /// Analisa posição interna (streaming parse + agrupamento por PV)
    pub async fn analyze(&mut self, depth: u8, multipv: usize) -> Result<Vec<AnalysisInfo>> {
        if let Some(tb) = &self.syzygy {
            let cnt = self.board.board().occupied().into_iter().count();
            if cnt <= 7 {
                let wdl = tb.probe_wdl(&self.board)?;
                let sc = match wdl {
                    AmbiguousWdl::Win  => Score::Mate(1),
                    AmbiguousWdl::Loss => Score::Mate(-1),
                    _                  => Score::Cp(0),
                };
                return Ok(vec![AnalysisInfo { score: sc, depth: 0, pv: Vec::new(), origin: AnalysisOrigin::Syzygy, multipv: 1 }]);
            }
        }
        // seta multipv apenas se mudou
        if multipv != self.current_multipv {
            self.cmd(&format!("setoption name MultiPV value {}", multipv)).await?;
            self.current_multipv = multipv;
        }
        // timeout global + por linha
        let fut = async {
            self.ensure_synced().await?;
            self.cmd(&format!("go depth {} multipv {}", depth, multipv)).await?;
            let mut map = HashMap::<Vec<ShakMove>, AnalysisInfo>::new();
            let mut line = String::new();
            loop {
                let n = {
                    let mut r = self.stdout.lock().await;
                    timeout(ENGINE_TIMEOUT, r.read_line(&mut line)).await??
                };
                if n == 0 || line.starts_with("bestmove") { break; }
                if line.starts_with("info ") && line.contains(" pv ") {
                    if let Some(info) = parse_info_line(&line, &self.board) {
                        let entry = map.entry(info.pv.clone()).or_insert_with(|| info.clone());
                        if info.depth > entry.depth || (info.depth == entry.depth && info.score > entry.score) {
                            *entry = info;
                        }
                    }
                }
                line.clear();
            }
            let mut res: Vec<_> = map.into_values().collect();
            res.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.multipv.cmp(&b.multipv)));
            Ok(res)
        };
        match timeout(ENGINE_TIMEOUT, fut).await {
            Ok(inner) => inner,
            Err(_)    => Err(anyhow!("Engine analyze global timeout")),
        }
    }
}

/// Parser UCI “info ... pv ...”
fn parse_info_line(line: &str, base: &Chess) -> Option<AnalysisInfo> {
    let mut parts = line.split_whitespace().peekable();
    let mut depth = 0;
    let mut score = None;
    let mut multipv = 1;
    while let Some(tok) = parts.next() {
        match tok {
            "depth"   => if let Some(d) = parts.next().and_then(|s| s.parse().ok()) { depth = d },
            "multipv" => if let Some(m) = parts.next().and_then(|s| s.parse().ok()) { multipv = m },
            "score"   => if let Some(kind) = parts.next() {
                if let Some(v) = parts.next().and_then(|s| s.parse().ok()) {
                    score = match kind {
                        "cp"   => Some(Score::Cp(v)),
                        "mate" => Some(Score::Mate(v)),
                        _       => None,
                    };
                }
            },
            "pv" => break,
            _    => {},
        }
    }
    let sc = score?;
    let mut tmp = base.clone();
    let pv = parts
        .filter_map(|u| UciMove::from_ascii(u.as_bytes()).ok())
        .filter_map(|uci| uci.to_move(&mut tmp).ok())
        .collect();
    Some(AnalysisInfo { score: sc, depth, pv, origin: AnalysisOrigin::Engine, multipv })
}
