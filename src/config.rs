// Configurações centralizadas para o extrator de puzzles de xadrez

// Configurações padrão para argumentos da linha de comando
pub const DEFAULT_DEPTH: u8 = 16;                  // Profundidade padrão para análise
pub const MAX_ALTERNATIVE_LINES: u8 = 2;           // Número máximo de linhas alternativas completas
pub const SOLVER_MIN_MOVES: u8 = 2;                // Mínimo de lances do resolvedor

// Para uma varredura ainda mais rápida com soluções muito profundas
pub const SCAN_DEPTH_MULTIPLIER: f32 = 1.0;        // Profundidade base para varredura
pub const SOLVE_DEPTH_MULTIPLIER: f32 = 1.2;       // 120% da profundidade base para resolver

// Limiares para determinar a qualidade/unicidade de puzzles
pub const PUZZLE_UNICITY_THRESHOLD: i32 = 200;     // Margem mínima para próximo lance pior (2 peões)
pub const BLUNDER_THRESHOLD: i32 = 150;            // Queda mínima na avaliação para detectar um blunder (1.5 peão)
pub const ALT_THRESHOLD: i32 = 25;                 // Diferença máxima (em cp) para considerar lances equivalentes (0.25 peão)
pub const MATE_ALT_THRESHOLD: i32 = 2;             // Diferença máxima de plies para mates
pub const COMPLETELY_WINNING_THRESHOLD: i32 = 500; // Limiar (em cp) para posição completamente ganha mesmo após erro (5 peões)
pub const HANGING_THRESHOLD: i32 = 400;            // Limite mínimo de diferença para identificar hanging piece

// Constantes de valor em peões para avaliações
pub const WINNING_ADVANTAGE: i32 = 150;            // Vantagem considerada decisiva (1.5 peão)
pub const DRAWING_RANGE: i32 = 100;                // Intervalo para considerar posição como aproximadamente igualada (-1 a +1)

// Valores para configuração do Stockfish
// Número de threads e tamanho de hash em MB usados no Stockfish
pub const THREADS: u32 = 4;
pub const HASH_MB: u32 = 1024;
