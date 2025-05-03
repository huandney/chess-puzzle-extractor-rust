#!/bin/bash
set -e

# Verifica se o executável 'stockfish' já existe no diretório atual
if [[ -x "./stockfish" ]]; then
    echo "Stockfish já está instalado. Pulando compilação." >&2
    exit 0
fi

# Clona o repositório oficial do Stockfish (repositório principal)
git clone --depth=1 -q https://github.com/official-stockfish/Stockfish.git Stockfish
cd Stockfish/src

# Compila o Stockfish (build otimizado padrão)
make build -j"$(nproc)" > /dev/null

# Baixa a rede neuronal (NNUE) padrão para o Stockfish
make net > /dev/null || true

# Move o binário compilado para o diretório raiz do projeto
cp stockfish ../../stockfish || mv stockfish ../../stockfish

# Move a rede neuronal, se baixada, para o diretório raiz do projeto
if ls nn-*.nnue &>/dev/null; then
    mv nn-*.nnue ../..
fi

# Limpa arquivos temporários de compilação
cd ../..
rm -rf Stockfish

echo "Stockfish compilado e instalado com sucesso."
exit 0
