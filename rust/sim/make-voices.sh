#!/usr/bin/env bash
# Generates the speech used by the simulator with macOS's built-in voices
# (no downloads): you, the far-end caller, and a background talker.
set -euo pipefail
cd "$(dirname "$0")/voices"
gen() { # name voice text
  say -v "$2" -o "$1.aiff" "$3"
  afconvert -f WAVE -d LEF32@48000 -c 1 "$1.aiff" "$1.wav"
  rm -f "$1.aiff"
}
gen you "Daniel" "Thanks everyone for joining. I wanted to walk through the launch plan for next week. The first build goes to the beta group on Monday, and if the crash rate stays low we expand to everyone on Thursday. I still need the final numbers from marketing, so please send those over before the end of the day. Does anyone have concerns about the timeline?"
gen caller "Flo (English (US))" "Hi, can you hear me okay? I think the timeline works, but the support team needs at least two days to prepare the help articles. Also, the Windows version is still behind, so we should mention that in the announcement. I can share the draft with you this afternoon if that helps."
gen background "Eddy (English (US))" "Did you see the game last night? I could not believe that last minute goal. Anyway, are we still going for lunch at noon? The new place on the corner is supposed to be really good, and I heard they have a great soup. Let me grab my jacket and we can head out after this meeting."
ls -l *.wav
