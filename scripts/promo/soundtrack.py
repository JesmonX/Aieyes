#!/usr/bin/env python3
"""Generate the promo soundtrack: an original ambient bed plus the app's own session chimes.

Pure standard library. Writes a 48 kHz stereo WAV; scripts/promo/build.sh converts it to AAC.
Usage: python3 scripts/promo/soundtrack.py out.wav [cues.json]
Without cues it scores the overview video. A composition exports its cue sheet with
node scripts/promo/capture.cjs --composition <name> --cues cues.json.
"""
import json
import math
from pathlib import Path
import random
import struct
import sys
import wave

ROOT = Path(__file__).resolve().parents[2]
# Cue sheet: arpeggio, louder arpeggio and rhythm spans, half-time spans, scene cuts,
# app sounds [time, file, gain], pointer clicks, key presses and notification pops.
CUES = dict(duration=99.0, arpeggio=[8.6, 95], bright=[14, 92], rhythm=[14, 91.5], sparse=[[48, 71]],
            cuts=[8.5, 14, 27, 37, 48, 71, 84, 92], sounds=[[48 + 3.2, "needs-input.wav", 0.75], [48 + 22.0, "complete.wav", 0.65]],
            clicks=[48 + c for c in [7, 12.4, 14.4, 16.4, 18.6, 19.2, 20.8]], keys=[], pops=[])
if len(sys.argv) > 2:
    CUES.update(json.loads(Path(sys.argv[2]).read_text()))
RATE, DURATION, BPM = 48000, float(CUES["duration"]), 96
BEAT = 60 / BPM
N = int(RATE * DURATION)
left, right = [0.0] * N, [0.0] * N


def add(start, samples, gain=1.0, pan=0.0):
    i0 = int(start * RATE)
    gl, gr = gain * math.cos((pan + 1) * math.pi / 4), gain * math.sin((pan + 1) * math.pi / 4)
    for k, v in enumerate(samples):
        i = i0 + k
        if 0 <= i < N:
            left[i] += v * gl; right[i] += v * gr


def hz(midi):
    return 440 * 2 ** ((midi - 69) / 12)


def pad(freqs, length, attack=1.2, release=1.6):
    n = int(length * RATE)
    out = [0.0] * n
    phases = [random.random() * 6.28 for _ in freqs for _ in range(3)]
    for k in range(n):
        t = k / RATE
        env = min(1, t / attack) * min(1, (length - t) / release)
        v = 0.0
        for j, f in enumerate(freqs):
            for d, detune in enumerate((-0.12, 0, 0.11)):
                v += math.sin(2 * math.pi * f * (1 + detune / 100) * t + phases[j * 3 + d]) * (0.6 if d == 1 else 0.35)
                v += 0.18 * math.sin(4 * math.pi * f * t)
        out[k] = v * env / (len(freqs) * 2.4)
    return out


def pluck(freq, length=0.9, decay=5.5):
    n = int(length * RATE)
    return [math.exp(-decay * k / RATE) * min(1, k / (0.004 * RATE)) *
            (math.sin(2 * math.pi * freq * k / RATE) + 0.25 * math.sin(4 * math.pi * freq * k / RATE) + 0.08 * math.sin(6 * math.pi * freq * k / RATE))
            for k in range(n)]


def kick():
    n = int(0.32 * RATE)
    out, phase = [], 0.0
    for k in range(n):
        t = k / RATE
        phase += 2 * math.pi * (48 + 90 * math.exp(-t * 28)) / RATE
        out.append(math.sin(phase) * math.exp(-t * 9))
    return out


def hat():
    n = int(0.06 * RATE)
    rnd = random.Random(7)
    prev, out = 0.0, []
    for k in range(n):
        x = rnd.uniform(-1, 1)
        out.append((x - prev) * math.exp(-k / RATE * 70)); prev = x
    return out


def swell(length=1.4):
    n = int(length * RATE)
    rnd = random.Random(3)
    out, lp = [], 0.0
    for k in range(n):
        p = k / n
        lp += (rnd.uniform(-1, 1) - lp) * (0.02 + 0.25 * p)
        out.append(lp * p ** 2 * (1 - p) ** 0.3 * 2.5)
    return out


def key(seed):
    """A soft keyboard tick: a short band-limited noise burst with a little body."""
    rnd = random.Random(seed)
    n, lp, out = int(0.028 * RATE), 0.0, []
    pitch = 1700 + rnd.uniform(-250, 250)
    for k in range(n):
        lp += (rnd.uniform(-1, 1) - lp) * 0.35
        out.append((lp * 0.8 + 0.5 * math.sin(2 * math.pi * pitch * k / RATE)) * math.exp(-k / RATE * 180))
    return out


def load_wav(name):
    with wave.open(str(ROOT / "apps/desktop/web/sounds" / name)) as w:
        rate, width, channels = w.getframerate(), w.getsampwidth(), w.getnchannels()
        raw = w.readframes(w.getnframes())
    count = len(raw) // width
    values = struct.unpack("<%dh" % count, raw) if width == 2 else [b - 128 for b in raw]
    mono = [values[i] / 32768 for i in range(0, count, channels)]
    return [mono[int(k * rate / RATE)] for k in range(int(len(mono) * RATE / rate))]


random.seed(11)
# Chords: Dmaj9 – Bm11 – Gmaj7 – A6sus, two bars each, voiced warm and low.
chords = [[50, 57, 62, 64, 66, 69], [47, 54, 59, 62, 64, 69], [43, 50, 55, 59, 62, 66], [45, 52, 57, 59, 62, 64]]
bar = BEAT * 4
t, i = 0.0, 0
while t < DURATION:
    add(t, pad([hz(m) for m in chords[i % 4]], bar * 2 + 1.4), 0.55)
    add(t, pad([hz(chords[i % 4][0] - 12)], bar * 2 + 1.0, 0.4, 1.0), 0.45)
    t += bar * 2; i += 1

# Arpeggio from the reveal onward; rhythm from the first feature through server monitoring.
scale = [62, 64, 66, 69, 71, 74, 76, 78]
step = BEAT / 2
for k in range(int(CUES["arpeggio"][0] / step), int(CUES["arpeggio"][1] / step)):
    start = k * step
    chord = chords[int(start / (bar * 2)) % 4]
    notes = sorted({(m % 12) for m in chord})
    pick = [n for n in scale if n % 12 in notes] or scale
    note = pick[(k * 3) % len(pick)]
    gain = 0.13 if CUES["bright"][0] <= start < CUES["bright"][1] else 0.08
    add(start, pluck(hz(note + (12 if k % 8 == 7 else 0))), gain, -0.45 if k % 2 else 0.45)
for b in range(int(CUES["rhythm"][0] / BEAT), int(CUES["rhythm"][1] / BEAT)):
    start = b * BEAT
    if b % 2 and any(a <= start < z for a, z in CUES["sparse"]):  # lighter during desktop scenes
        continue
    add(start, kick(), 0.30)
    add(start + BEAT / 2, hat(), 0.05, 0.3)

# Transition swells, and a soft low hit on each scene cut.
for cut in CUES["cuts"]:
    add(cut - 1.4, swell(), 0.10)
    add(cut, pluck(hz(38), 1.8, 2.2), 0.35)

# Product sounds: the app's own session tones at the pet's state changes.
for start, name, gain in CUES["sounds"]:
    add(start, load_wav(name), gain)
# Pointer clicks, typing and notification banners.
for click in CUES["clicks"]:
    add(click, [math.exp(-k / RATE * 900) * math.sin(2 * math.pi * 2300 * k / RATE) for k in range(int(0.03 * RATE))], 0.18)
for index, start in enumerate(CUES["keys"]):
    add(start, key(index), 0.07, -0.15)
for start in CUES["pops"]:
    add(start, pluck(hz(88), 0.5, 9), 0.16, 0.5)
    add(start + 0.09, pluck(hz(93), 0.7, 7), 0.14, 0.5)

# Fade in and out, gentle glue compression, normalize.
peak = 0.0
for k in range(N):
    t = k / RATE
    fade = min(1, t / 1.5) * min(1, (DURATION - t) / 2.5)
    for ch in (left, right):
        v = ch[k] * fade
        ch[k] = math.tanh(v * 1.4) / 1.4
        peak = max(peak, abs(ch[k]))
scale = 0.84 / peak
with wave.open(sys.argv[1], "wb") as out:
    out.setnchannels(2); out.setsampwidth(2); out.setframerate(RATE)
    out.writeframes(b"".join(struct.pack("<hh", int(l * scale * 32767), int(r * scale * 32767)) for l, r in zip(left, right)))
print("Wrote", sys.argv[1])
