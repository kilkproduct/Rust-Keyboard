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

# The swipe assets only need the three models SwipeDecoderDictionary names:
# honorable_sturgeon (encoder), chosen_planarian (English language model) and
# magic_macaw (decoder), plus scoring.json. The GIF is documentation and
# hungry_jellyfish is an unused second language model.
swipe_dir="$root/java/assets/futo-swipe"
if [ -d "$swipe_dir" ]; then
  for unused in animations hungry_jellyfish; do
    if [ -e "$swipe_dir/$unused" ]; then
      echo "stripping futo-swipe/$unused"
      rm -rf "${swipe_dir:?}/$unused"
    fi
  done
else
  echo "no $swipe_dir, nothing to strip"
fi

du -sh "$res_dir" "$swipe_dir" 2>/dev/null || true
