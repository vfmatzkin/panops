#!/usr/bin/env bash
# Regenerate every fixture from source. Idempotent. Run from repo root.
set -euo pipefail

cd "$(dirname "$0")/../../.."

AUDIO=tests/fixtures/audio
VIDEO=tests/fixtures/video
SHOTS=tests/fixtures/screenshots

WORK=$(mktemp -d -t panops)
trap 'rm -rf "$WORK"' EXIT

echo "[1/5] EN audio"
say -v Samantha -r 130 -o "$WORK/en.aiff" -f "$AUDIO/en_30s.transcript.txt"
ffmpeg -y -loglevel error -i "$WORK/en.aiff" -ar 16000 -ac 1 -c:a pcm_s16le "$AUDIO/en_30s.wav"

echo "[2/5] ES audio"
VOICES=$(say -v "?")
ES_VOICE=""
for v in "Mónica" "Paulina" "Jorge" "Diego"; do
  if printf '%s\n' "$VOICES" | grep -qE "^${v} +es_"; then ES_VOICE="$v"; break; fi
done
if [ -z "$ES_VOICE" ]; then
  ES_VOICE=$(printf '%s\n' "$VOICES" | awk '/ es_[A-Z]+ +#/ { sub(/ +es_.*/, ""); print; exit }')
fi
if [ -z "$ES_VOICE" ]; then
  cat <<'MSG' >&2
ERROR: No Spanish voice installed in macOS.

Install one:
  1. Spoken Content -> System Voice -> Manage Voices...
  2. Expand "Spanish", check "Mónica" (or any es_*) -> Done.
  3. Re-run this script.

Opening System Settings now.
MSG
  open "x-apple.systempreferences:com.apple.preference.universalaccess?Spoken_Content"
  exit 1
fi
echo "  using voice: $ES_VOICE"
say -v "$ES_VOICE" -r 130 -o "$WORK/es.aiff" -f "$AUDIO/es_30s.transcript.txt"
ffmpeg -y -loglevel error -i "$WORK/es.aiff" -ar 16000 -ac 1 -c:a pcm_s16le "$AUDIO/es_30s.wav"

echo "[3/5] Mixed audio"
ffmpeg -y -loglevel error \
  -i "$AUDIO/en_30s.wav" -i "$AUDIO/es_30s.wav" \
  -filter_complex "[0:a][1:a]concat=n=2:v=0:a=1[out]" \
  -map "[out]" -c:a pcm_s16le "$AUDIO/mixed_60s.wav"

echo "[4/5] Video and screenshots"
ffmpeg -y -loglevel error \
  -f lavfi -i "color=c=red:s=1280x720:d=10:r=30" \
  -f lavfi -i "color=c=blue:s=1280x720:d=10:r=30" \
  -f lavfi -i "color=c=green:s=1280x720:d=10:r=30" \
  -f lavfi -i "color=c=yellow:s=1280x720:d=10:r=30" \
  -f lavfi -i "color=c=purple:s=1280x720:d=10:r=30" \
  -f lavfi -i "color=c=orange:s=1280x720:d=10:r=30" \
  -filter_complex "[0:v][1:v][2:v][3:v][4:v][5:v]concat=n=6:v=1:a=0[outv]" \
  -map "[outv]" -c:v libx264 -pix_fmt yuv420p -preset fast \
  "$VIDEO/screen_60s.mp4"

rm -f "$SHOTS"/*.jpg
ffmpeg -y -loglevel error -i "$VIDEO/screen_60s.mp4" \
  -vf "fps=1/5,scale=1280:720" -q:v 3 "$SHOTS/%03d.jpg"

echo "[5/5] Multi-speaker audio (Samantha -> Daniel -> Samantha)"
cat > "$WORK/turn1.txt" <<'TURN'
Welcome to this meeting. Let's go over the agenda for today. We have several important items to discuss in the next sixty minutes together.
TURN
cat > "$WORK/turn2.txt" <<'TURN'
Thanks for the introduction. The first item is the budget review for next quarter. We need to approve the spending plan before the end of this week.
TURN
cat > "$WORK/turn3.txt" <<'TURN'
Right. I'll start with the marketing line items, then move to engineering, and finally we will cover any remaining operations expenses for the team.
TURN
say -v Samantha -r 165 -o "$WORK/turn1.aiff" -f "$WORK/turn1.txt"
say -v Daniel   -r 165 -o "$WORK/turn2.aiff" -f "$WORK/turn2.txt"
say -v Samantha -r 165 -o "$WORK/turn3.aiff" -f "$WORK/turn3.txt"
for i in 1 2 3; do
  ffmpeg -y -loglevel error -i "$WORK/turn$i.aiff" -ar 16000 -ac 1 -c:a pcm_s16le "$WORK/turn${i}_raw.wav"
  ffmpeg -y -loglevel error \
    -i "$WORK/turn${i}_raw.wav" \
    -af "apad=whole_dur=20" \
    -t 20 \
    -c:a pcm_s16le \
    "$WORK/turn$i.wav"
done
ffmpeg -y -loglevel error \
  -i "$WORK/turn1.wav" -i "$WORK/turn2.wav" -i "$WORK/turn3.wav" \
  -filter_complex "[0:a][1:a][2:a]concat=n=3:v=0:a=1[out]" \
  -map "[out]" -c:a pcm_s16le \
  "$AUDIO/multi_speaker_60s.wav"

echo "Done. Run 'git status tests/fixtures/' to see what changed."
