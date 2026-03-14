# Voice Pipeline — Algorithm & Logic

## Overview

The gclaw-voice pipeline processes audio through 5 stages:

```
Microphone → Ring Buffer → VAD → STT → Wake Word / User Speech
                                  ↓
                            Brain (via IPC)
                                  ↓
                         TTS → Speaker (with barge-in monitoring)
```

## 1. Audio Capture

**Module**: `audio/capture.rs`

- **Library**: cpal (cross-platform: WASAPI/CoreAudio/ALSA/Android)
- **Format**: 16kHz, mono, f32 samples
- **Buffer**: 30-second ring buffer (480,000 samples)
- **Thread model**: cpal callback writes to ring buffer; main loop reads from it
- **Latency**: Sub-1ms callback overhead

```
┌──────────┐    callback    ┌───────────────┐
│ cpal     │ ──────────────►│ Ring Buffer   │
│ stream   │  f32 samples   │ (30s, 480K)   │
└──────────┘                └───────┬───────┘
                                    │ read_last()
                                    ▼
                              VAD processing
```

### Ring Buffer Design

Circular buffer with wrap-around. Write position advances monotonically.
`read_last(n)` returns the most recent n samples by walking backwards from write position.

## 2. Voice Activity Detection (VAD)

**Module**: `vad/silero.rs`

- **Model**: Silero VAD v5 (ONNX, ~2MB)
- **Runtime**: ONNX Runtime via `ort` crate (dynamic loading)
- **Frame size**: 512 samples (32ms @ 16kHz)
- **Output**: Speech probability [0.0, 1.0]

### State Machine

```
┌───────────────┐  speech prob > 0.5  ┌──────────────┐
│ WaitingFor    │ ───────────────────► │ InSpeech     │
│ Speech        │                      │              │
│               │ ◄─────────────────── │ speech < 5   │
│               │  discard (too short) │ frames: noise│
└───────────────┘                      └──────┬───────┘
                                              │
                                    15 frames silence
                                    or max duration
                                              │
                                              ▼
                                    ┌──────────────────┐
                                    │ SpeechEnd        │
                                    │ (return segment) │
                                    └──────────────────┘
```

### Parameters

| Parameter | Value | Effect |
|-----------|-------|--------|
| `SPEECH_THRESHOLD` | 0.5 | Probability cutoff for speech detection |
| `MIN_SPEECH_FRAMES` | 5 (160ms) | Minimum utterance length (debounce) |
| `MAX_SILENCE_FRAMES` | 15 (480ms) | Pause threshold (matches speech.py 0.5s) |
| `MAX_SPEECH_FRAMES` | 1875 (60s) | Safety cap on utterance length |

### LSTM State

Silero VAD v5 uses LSTM layers with state tensors `h` (hidden) and `c` (cell), each shaped `[2, 1, 64]`. State persists across frames within an utterance, resets between utterances.

## 3. Speech-to-Text (STT)

**Module**: `stt/whisper.rs`

- **Engine**: whisper.cpp via `whisper-rs`
- **Models**: tiny (75MB), base (150MB), small (500MB), medium (1.5GB)
- **Input**: f32 audio from VAD SpeechSegment
- **Output**: Transcription { text, language, confidence }
- **Language**: Auto-detect or hinted

### Whisper Parameters

```
SamplingStrategy: Greedy { best_of: 1 }
Threads: 4
SingleSegment: true       # Process as one utterance
NoContext: true            # Don't use prior context (fresh each time)
SuppressBlank: true        # Filter empty segments
PrintTimestamps: false     # Speed optimization
```

### Noise Filtering

Post-STT filter removes common Whisper hallucinations:

- Text < 2 chars
- Known hallucinations: "Thank you.", "Thanks for watching.", "The end.", "..."
- All-punctuation output
- Confidence < 0.1

## 4. Wake Word Detection

**Module**: `wake/detector.rs`

- **Method**: Fuzzy string matching (Jaro-Winkler via `strsim`)
- **Threshold**: 0.78 (whole phrase), 0.95 (single-char word-level)
- **Wake words**: Generated from AI name ("hey G", "ok G", "yo G", etc.)

### Flow

```
VAD detects 2s speech clip
        │
        ▼
Whisper transcribes to text
        │
        ▼
For each wake word variant:
  1. Exact substring match? → WAKE
  2. Jaro-Winkler ≥ 0.78?  → WAKE
        │
        ▼ (no match)
For each word in text (short names only):
  Jaro-Winkler ≥ 0.95?     → WAKE
        │
        ▼ (no match)
Continue listening
```

### Mishearing Variants

For short AI names (≤2 chars), common Whisper mishearings are added:
- "G" → "gee", "ji", "jee", "hey gee", "hey ji"
- "J" → "jay", "hey jay"

## 5. TTS (Text-to-Speech)

**Module**: `tts/piper.rs`, `tts/espeak.rs`

### Engine Selection

```
Language = "en"  → Piper (neural, fast, high quality)
Language = other → espeak-ng (rule-based, multilingual)
Neither available → error (no fallback to cloud TTS)
```

### Piper TTS

- **Execution**: Subprocess (`piper --model voice.onnx --output-raw`)
- **Input**: Text via stdin
- **Output**: Raw 16-bit PCM at 22050Hz via stdout
- **Conversion**: i16 → f32 normalization
- **Models**: en_US-lessac-medium recommended (~50MB)

### espeak-ng

- **Execution**: Subprocess (`espeak-ng --stdout -v <lang> "<text>"`)
- **Output**: WAV file via stdout (44-byte header skipped)
- **Languages**: 100+ supported
- **Quality**: Lower than Piper but universal coverage

## 6. Barge-In

**Module**: `bargein/controller.rs`

Enables users to interrupt long TTS responses naturally.

### Algorithm

```
1. Start TTS playback with stop_flag = false
2. Wait 200ms (avoid detecting our own speech)
3. While playing:
   a. Monitor stop_flag every 32ms
   b. If speech detected → set stop_flag = true
4. If playback completed normally → return Completed
5. If stopped:
   a. Wait 500ms for user to finish speaking
   b. Capture last 3s of audio
   c. Transcribe via Whisper
   d. If noise → return Completed
   e. Else → return Interrupted(text)
```

### Concurrency Model

```
Main Thread:  play_blocking(samples, stop_flag)
Monitor Thread: sleep(200ms) then poll stop_flag every 32ms
                ↓
           stop_flag = true → playback fills with silence
```

## 7. Main Loop State Machine

**Module**: `shell.rs`

```
┌──────────┐  wake word   ┌──────────┐
│   IDLE   │ ────────────►│  ACTIVE  │
│          │              │          │
│ VAD+STT  │ ◄────────────│ 90s idle │
│ wake     │  auto-sleep  │ VAD+STT  │
│ detect   │              │ IPC recv │
└──────────┘              └──────────┘
```

### IDLE Mode
1. Read 512-sample frame from ring buffer
2. Run VAD → accumulate speech
3. On SpeechEnd: transcribe with Whisper
4. Check wake word match
5. If matched: send WakeWordDetected IPC, transition to ACTIVE

### ACTIVE Mode
1. Check auto-sleep (90s inactivity)
2. `tokio::select!`:
   - VAD processing: detect speech → transcribe → send UserSpeech IPC
   - IPC receive: handle Speak/SpeakInterruptible/StopSpeaking/Shutdown

### Performance Budget

| Operation | Budget | Actual |
|-----------|--------|--------|
| VAD frame | 32ms | <1ms |
| STT (2s clip, base model) | 500ms | ~200ms |
| TTS (sentence, Piper) | 200ms | ~100ms |
| Wake word match | 1ms | <0.1ms |
| IPC round-trip | 5ms | <1ms |
| End-to-end (wake → response) | 2000ms | ~800ms |
