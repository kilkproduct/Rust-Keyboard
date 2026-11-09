#!/usr/bin/env bash
# English only build: drop the resources that belong to other languages, and
# assets this app never reads.
#
# java/res-large and java/assets/futo-swipe are shared submodules, so the trim
# happens on a checkout instead of inside those repositories. Run this before
# Gradle; locally you can skip it, the package is just bigger.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# One compiled dictionary per language; the keyboard only offers English.
res_dir="$root/java/res-large/raw"
if [ -d "$res_dir" ]; then
  for dict in "$res_dir"/main_*.dict; do
    [ -e "$dict" ] || continue
    case "$(basename "$dict")" in
      main_en.dict) ;;
      *) echo "stripping $(basename "$dict")"; rm -f "$dict" ;;
    esac
  done
else
  echo "no $res_dir, nothing to strip"
fi

# The transformer language model: a multilingual model that predicted words in
# other languages, ~75 MB of package and hundreds of megabytes of RAM. The
# engine now predicts from the English dictionary alone.
if [ -e "$res_dir/ml4_q6_k.gguf" ]; then
  echo "stripping the transformer language model"
  rm -f "$res_dir"/ml4_q6_k.gguf "$res_dir"/ml4_*.gguf
fi

# Swipe typing is gone: the three TFLite models (encoder, English language
# model, decoder) and the GIF are ~30 MB of assets nothing reads any more.
swipe_dir="$root/java/assets/futo-swipe"
if [ -d "$swipe_dir" ]; then
  echo "stripping futo-swipe entirely"
  rm -rf "${swipe_dir:?}"/*
else
  echo "no $swipe_dir, nothing to strip"
fi

du -sh "$res_dir" "$swipe_dir" 2>/dev/null || true
