#!/usr/bin/env bash

set -euo pipefail

function chapterize() {
    local input_path
    local audio_filename
    local audio_name
    local out_dir

    input_path="$1"
    audio_filename="$(basename "$input_path")"
    audio_name="${audio_filename%.*}"
    out_dir="${2:-.}/${audio_name}"

    mkdir -p "$out_dir"

    echo "Chapterizing ${input_path}"
    echo "Output dir: ${out_dir}"

    RUST_BACKTRACE=full \
    ./target/debug/audiobook-chapterizer -vv --model ./vosk-model-en-us-0.22 \
        --write_matches "${out_dir}/${audio_name}.jsonl" \
        -i "$input_path" \
        "${out_dir}/${audio_name}.cue" \
        2>&1 | tee "${out_dir}/${audio_name}.log"

    echo "Done chapterizing ${input_path}"
}

just build

while read -r file_path; do
    set +e
    chapterize "$file_path" ./output
    set -e
done < ./chapterize.txt
