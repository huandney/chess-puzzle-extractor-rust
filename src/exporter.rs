// src/exporter.rs
// Exporta puzzles gerados para arquivos PGN de saída

// Biblioteca padrão
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

// Bibliotecas externas
use anyhow::{Context, Result};
use log::{debug, info, warn, trace};

/// Abre o arquivo de saída para escrita dos puzzles.
/// Se `resume` for true e o arquivo já existir, abre em modo de acréscimo (append) para continuar escrevendo.
/// Caso contrário, cria um novo arquivo (sobrescreve se já existir).
pub fn open_output_file(path: &Path, resume: bool) -> Result<File> {
    info!("open_output_file: abrindo arquivo de saída: {:?}, resume={}", path, resume);

    if resume && path.exists() {
        debug!("open_output_file: arquivo existe e resume=true, abrindo em modo append");
        OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .context("Falha ao abrir arquivo de saída para acrescentar dados")
    } else {
        if path.exists() && !resume {
            info!("open_output_file: arquivo existe mas resume=false, sobrescrevendo");
        } else {
            debug!("open_output_file: criando novo arquivo");
        }

        File::create(path).context("Falha ao criar arquivo de saída")
    }
}

/// Exporta um puzzle para o arquivo de saída
pub fn export_puzzle(pgn_string: &str, output: &mut dyn Write) -> Result<()> {
    debug!("export_puzzle: exportando puzzle com {} caracteres", pgn_string.len());

    // Extraindo informações básicas para o log
    let primeiro_lance = pgn_string
        .lines()
        .find(|line| !line.starts_with('[') && !line.is_empty())
        .unwrap_or("(não encontrado)");

    let fen = pgn_string
        .lines()
        .find(|line| line.starts_with("[FEN"))
        .unwrap_or("(FEN não encontrado)");

    let fase = pgn_string
        .lines()
        .find(|line| line.starts_with("[Phase"))
        .unwrap_or("(Phase não encontrada)");

    let tatico = pgn_string
        .lines()
        .find(|line| line.starts_with("[Tactical"))
        .unwrap_or("(Tactical não encontrado)");

    info!("export_puzzle: salvando puzzle - {}, {}, {}", fase, tatico, primeiro_lance);
    trace!("export_puzzle: FEN inicial - {}", fen);

    writeln!(output, "{}", pgn_string).context("Falha ao escrever puzzle no arquivo de saída")?;
    writeln!(output).context("Falha ao escrever quebra de linha no arquivo de saída")?;

    debug!("export_puzzle: puzzle exportado com sucesso");
    Ok(())
}
