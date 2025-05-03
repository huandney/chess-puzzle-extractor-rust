// src/utils.rs
// ---------------------------------------------------------------------------
// Utilitários de PGN, arquivos e profundidades (independentes de Score).
// ---------------------------------------------------------------------------

use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions},
    io::BufReader,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{anyhow, Context, Result};
use indexmap::IndexMap;
use log::{trace, warn};
use pgn_reader::{BufferedReader, RawHeader, SanPlus, Skip, Visitor};
use shakmaty::{san::San, fen::Fen, CastlingMode, Chess, Color, Move, Position};

use crate::{
    builder::PuzzleSeq,
    config,
    engine::Engine,
};

// ---------------------------------------------------------------------------
// Contador rápido de jogos - implementação do Visitor para contar jogos em PGN
// ---------------------------------------------------------------------------
struct GameCounter { n: usize }

impl Visitor for GameCounter {
    type Result = ();
    fn begin_game(&mut self) { self.n += 1; if self.n % 1000 == 0 { trace!("… lendo jogo #{}", self.n); } }
    fn header(&mut self, _: &[u8], _: RawHeader<'_>) {}
    fn san(&mut self, _: SanPlus) {}
    fn begin_variation(&mut self) -> Skip { Skip(true) }
    fn end_variation(&mut self) {}
    fn end_game(&mut self) -> Self::Result {}
}

/// Conta o número total de jogos em um arquivo PGN
pub fn count_games(path: &Path) -> Result<u64> {
    let f = File::open(path).context("abrir PGN")?;
    let mut rdr = BufferedReader::new(BufReader::new(f));
    let mut c = GameCounter { n: 0 };
    while rdr.read_game(&mut c)? != None {}
    Ok(c.n as u64)
}

// ---------------------------------------------------------------------------
// MoveRecord & iterate_games - iteração preguiçosa sobre lances de uma partida
// ---------------------------------------------------------------------------
/// Estrutura que representa um lance dentro do arquivo PGN
#[derive(Debug, Clone)]
pub struct MoveRecord {
    pub game_idx: u32,                      // Índice do jogo no arquivo
    pub move_idx: u32,                      // Número do lance no jogo
    pub side:     Color,                    // Cor que executa o lance
    pub board:    Chess,                    // Posição antes do lance
    pub san:      String,                   // Notação algébrica do lance
    pub mv:       Move,                     // Movimento em formato interno
    pub headers:  Vec<(String,String)>,     // Headers do PGN do jogo
}

/// Iterador preguiçoso de lances do PGN - processa um jogo por vez
pub fn iterate_games(path: &Path) -> Result<impl Iterator<Item=MoveRecord>> {
    let file = File::open(path).with_context(|| format!("abrir {}", path.display()))?;
    let reader = BufferedReader::new(BufReader::new(file));

    // Estado do iterador
    struct St<R: std::io::Read> {
        rdr: BufferedReader<R>,             // Leitor do arquivo
        q  : VecDeque<MoveRecord>,          // Fila de lances a processar
        idx: u32,                           // Índice do jogo atual
    }

    // Visitor para processar um jogo
    struct V<'a> {
        b  : Chess,                         // Tabuleiro atual
        mi : u32,                           // Índice do lance
        hdr: Vec<(String,String)>,          // Headers coletados
        q  : &'a mut VecDeque<MoveRecord>,  // Referência para fila de saída
        gi : u32,                           // Índice do jogo
    }

    impl<'a> V<'a> {
        fn new(q: &'a mut VecDeque<MoveRecord>, gi: u32) -> Self {
            Self { b: Chess::default(), mi:0, hdr:Vec::new(), q, gi }
        }
    }

    // Implementação do visitor para processar lances
    impl<'a> Visitor for V<'a> {
        type Result = ();
        fn begin_game(&mut self){ self.b = Chess::default(); self.mi=0; self.hdr.clear(); }

        // Coleta headers do PGN
        fn header(&mut self,n:&[u8],v:RawHeader<'_>){
            if let (Ok(k),Ok(val))=(std::str::from_utf8(n), std::str::from_utf8(v.as_bytes())) {
                self.hdr.push((k.into(), val.trim_matches('"').into()));
            }
        }

        // Processa cada lance e adiciona à fila
        fn san(&mut self, sp:SanPlus){
            if let Ok(mv)=sp.san.to_move(&self.b){
                self.mi+=1;
                self.q.push_back(MoveRecord{
                    game_idx:self.gi, move_idx:self.mi, side:self.b.turn(),
                    board:self.b.clone(), san:sp.san.to_string(), mv:mv.clone(), headers:self.hdr.clone()
                });
                self.b.play_unchecked(&mv);
            }
        }
        fn end_game(&mut self){}
    }

    // Inicializa estado
    let mut st = St { rdr: reader, q: VecDeque::new(), idx: 0 };

    // Retorna o iterador
    Ok(std::iter::from_fn(move || loop {
        // Se tem lance na fila, retorna
        if let Some(r)=st.q.pop_front(){return Some(r);}

        // Senão, lê o próximo jogo
        st.idx+=1;
        let mut v = V::new(&mut st.q, st.idx);
        match st.rdr.read_game(&mut v){
            Ok(Some(_))=>continue,            // Jogo lido: volta para emitir lances
            Ok(None)=>return None,            // Fim do arquivo: termina
            Err(e)=>{ warn!("erro lendo jogo {}: {}", st.idx,e); continue; } // Erro: pula jogo
        }
    }))
}

// ---------------------------------------------------------------------------
// Profundidades para análise
// ---------------------------------------------------------------------------
/// Conjunto de profundidades para fases distintas de análise
pub struct DepthSet { pub scan: u8, pub solve: u8 }

/// Calcula as profundidades para scan e solve com base na profundidade base
pub fn calculate_depths(base: u8) -> DepthSet {
    DepthSet {
        scan : (base as f32 * config::SCAN_DEPTH_MULTIPLIER ).max(1.0) as u8,  // Profundidade para varredura
        solve: (base as f32 * config::SOLVE_DEPTH_MULTIPLIER).max(1.0) as u8,  // Profundidade para solução
    }
}

// ---------------------------------------------------------------------------
// Engine helper - preparação do motor
// ---------------------------------------------------------------------------
/// Prepara o motor de xadrez com as profundidades calculadas
pub async fn prepare_engine(base: u8) -> Result<(DepthSet, Engine)> {
    let depths = calculate_depths(base);
    let path   = detect_stockfish_path()?;
    let eng    = Engine::new(&path).await?;
    Ok((depths, eng))
}

// ---------------------------------------------------------------------------
// I/O helpers - formatação e verificação de arquivos
// ---------------------------------------------------------------------------
/// Formata tamanho de arquivo em B, KB ou MB
pub fn format_size(path: &Path) -> Result<String>{
    let b = fs::metadata(path)?.len();
    Ok(if b<1024{format!("{b} B")}
       else if b<1_048_576{format!("{:.2} KB", b as f64/1024.0)}
       else{format!("{:.2} MB", b as f64/1_048_576.0)})
}

/// Garante que um diretório exista, criando-o se necessário
pub fn ensure_dir_exists(dir:&Path)->Result<()>{
    if dir.exists(){return Ok(());}
    fs::create_dir_all(dir).with_context(||format!("criar {}",dir.display()))
}

/// Detecta caminho do executável Stockfish
pub fn detect_stockfish_path()->Result<String>{
    let local=PathBuf::from("./stockfish");
    if local.exists(){return Ok(local.to_string_lossy().into());}
    if Command::new("stockfish").arg("--version").output().is_ok(){return Ok("stockfish".into());}
    Err(anyhow!("Stockfish não encontrado"))
}

// ---------------------------------------------------------------------------
// Arquivo de saída - preparação do arquivo para exportação de puzzles
// ---------------------------------------------------------------------------
/// Prepara e abre o arquivo de saída para os puzzles
pub fn prepare_output_file(input:&PathBuf, out:Option<&PathBuf>, resume:bool)->Result<(PathBuf,File)>{
    // Define caminho de saída: usa fornecido ou constrói padrão
    let path = out.cloned().unwrap_or_else(||{
        let stem=input.file_stem().and_then(|s|s.to_str()).unwrap_or("output");
        let dir = PathBuf::from("puzzles");
        let _ = ensure_dir_exists(&dir);
        dir.join(format!("{stem}_puzzles.pgn"))
    });

    // Garante que diretório pai exista
    if let Some(p)=path.parent(){ensure_dir_exists(p)?;}

    // Abre arquivo com opções adequadas (append se resume, truncate se novo)
    let f = OpenOptions::new().write(true).create(true).append(resume).truncate(!resume).open(&path)?;
    Ok((path,f))
}

// ---------------------------------------------------------------------------
// Build PGN - constrói representação PGN final do puzzle
// ---------------------------------------------------------------------------
/// Constrói string PGN a partir de headers e sequência de lances
pub fn build_pgn_san<K,V>(hdr:&IndexMap<K,V>, seq:&PuzzleSeq)->Result<String>
where K:AsRef<str>, V:AsRef<str>{
    // Cabeçalhos PGN
    let mut pgn=String::new();
    for (k,v) in hdr { pgn.push_str(&format!("[{} \"{}\"]\n",k.as_ref(),v.as_ref())); }
    pgn.push('\n');

    // Inicializa tabuleiro a partir do FEN, se disponível
    let mut board = if let Some(fen)=hdr.iter().find_map(|(k,v)|
            k.as_ref().eq_ignore_ascii_case("fen").then(||v.as_ref())){
        fen.parse::<Fen>()?.into_position(CastlingMode::Standard)?
    }else{Chess::default()};

    // Linha principal de lances
    let init=board.turn();
    for (i,mv) in seq.moves.iter().enumerate(){
        // Numeração correta baseada no turno inicial
        if i==0{
            pgn.push_str(&format!("1{} ", if init==Color::White{'.'}else{'…'}));
        }else if (init==Color::White && i%2==0)||(init==Color::Black && i%2==1){
            pgn.push_str(&format!("{}. ", i/2+1));
        }

        // Adiciona o lance em SAN
        pgn.push_str(&format!("{} ", San::from_move(&board,mv)));
        board.play_unchecked(mv);
    }

    // Variantes alternativas
    let main=board.clone();
    for var in &seq.alternatives{
        pgn.push('(');
        let mut b=main.clone();
        for mv in var{
            pgn.push_str(&format!("{} ", San::from_move(&b,mv)));
            b.play_unchecked(mv);
        }
        if pgn.ends_with(' '){pgn.pop();}
        pgn.push_str(") ");
    }

    // Retorna PGN formatado
    Ok(pgn.trim_end().into())
}
