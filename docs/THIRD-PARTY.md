# Third-party models and libraries

What Capricorn uses, and under what terms. Code dependencies not listed here are ordinary crates and npm packages
under MIT or Apache-2.0 (see `Cargo.lock` and `ui/package-lock.json`).

## Models (downloaded on her first start, never committed)

| Model | Used for | License | Source |
|---|---|---|---|
| NVIDIA Parakeet TDT 0.6B v3, int8 (sherpa-onnx export) | speech to text | CC-BY-4.0 | k2-fsa/sherpa-onnx releases (or its Hugging Face copy), `sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8` |
| Silero VAD | where sentences start and end | MIT | k2-fsa/sherpa-onnx releases, `silero_vad.onnx` |
| Supertonic 3, int8 (Supertone) | her voice (optional) | code MIT, weights OpenRAIL-M (use restrictions against harmful uses) | k2-fsa/sherpa-onnx releases, `sherpa-onnx-supertonic-3-tts-int8-2026-05-11` |
| Kokoro v1.0 (hexgrad), full precision | her voice (default, speaker jf_alpha) | Apache-2.0 | k2-fsa/sherpa-onnx releases (or its Hugging Face copy), `kokoro-multi-lang-v1_0` |
| Kokoro v1.0, int8 (hexgrad) | her voice, smaller (optional) | Apache-2.0 | k2-fsa/sherpa-onnx releases, `kokoro-int8-multi-lang-v1_0` |
| KittenTTS mini v0.8 (KittenML) | her voice (optional) | Apache-2.0 | k2-fsa/sherpa-onnx releases, `kitten-mini-en-v0_8` |
| Ollama models (default `qwen3-vl:8b-instruct`) | her local brain and eyes | each model's own (Qwen3-VL: Apache-2.0) | pulled by Ollama, never shipped |

## Libraries of note

| Library | Used for | License |
|---|---|---|
| sherpa-onnx (k2-fsa), with ONNX Runtime | running the speech models | Apache-2.0 (ONNX Runtime: MIT) |
| Tauri 2 | the app shell | MIT / Apache-2.0 |
| PixiJS 8 | drawing her | MIT |
| cpal | the microphone | Apache-2.0 |
| windows-rs | Win32 and WinRT calls | MIT / Apache-2.0 |
| Chakra Petch, JetBrains Mono (via Fontsource) | type | SIL Open Font License 1.1 |

## Services (only when used)

| Service | Used for | Notes |
|---|---|---|
| Open-Meteo | weather and place names | free, no key, no account |
| wttr.in | weather fallback | free, no key |
| Discord | messaging her from your phone (Settings › Phone) | only with your own bot's token, only when you turn it on |
| Anthropic, OpenAI, Gemini, OpenRouter, Groq, DeepSeek, Mistral, NVIDIA, Ollama Cloud | her online brain | only with your key or plan, only when you pick one |
