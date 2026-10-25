#!/usr/bin/env python3
"""Generate Aieyes's original, short PCM notification tones without external assets."""
import io
import math
from pathlib import Path
import struct
import sys
import wave

ROOT = Path(__file__).resolve().parents[1]
RATE = 22050


def note(t, frequency, decay=9, harmonics=(1, 0.16, 0.04)):
    if t < 0:
        return 0
    attack = min(1, t / 0.008)
    return attack * math.exp(-decay * t) * sum(
        amplitude * math.sin(2 * math.pi * frequency * (index + 1) * t)
        for index, amplitude in enumerate(harmonics)
    )


TONES = {
    'bell': (0.62, lambda t: note(t, 880, 8, (1, 0.3, 0.12))),
    'chime': (0.70, lambda t: sum(note(t - delay, frequency, 8) for delay, frequency in [(0, 523.25), (0.07, 659.25), (0.14, 783.99)])),
    'marimba': (0.56, lambda t: note(t, 440, 15, (1, 0.05, 0.3)) + note(t - 0.16, 659.25, 15, (1, 0.05, 0.3))),
    'droplet': (0.48, lambda t: min(1, t / 0.006) * math.exp(-12 * t) * math.sin(2 * math.pi * (1050 * t - 420 * t * t))),
    'pulse': (0.42, lambda t: note(t, 659.25, 16, (1, 0, 0.12, 0, 0.04)) + note(t - 0.13, 880, 16, (1, 0, 0.12, 0, 0.04))),
}


def render(duration, sample):
    values = [sample(index / RATE) * min(1, (duration - index / RATE) / 0.025) for index in range(round(duration * RATE))]
    peak = max(map(abs, values))
    pcm = b''.join(struct.pack('<h', round(value / peak * 0.45 * 32767)) for value in values)
    output = io.BytesIO()
    with wave.open(output, 'wb') as wav:
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(RATE)
        wav.writeframes(pcm)
    return output.getvalue()


for name, (duration, sample) in TONES.items():
    target = ROOT / 'apps/desktop/web/sounds' / (name + '.wav')
    data = render(duration, sample)
    if '--check' in sys.argv:
        assert target.read_bytes() == data, f'Stale sound: {target}; run scripts/generate-session-sounds.py'
    else:
        target.write_bytes(data)
print(f'Original session tones: {len(TONES)} assets verified' if '--check' in sys.argv else f'Generated {len(TONES)} original session tones')
