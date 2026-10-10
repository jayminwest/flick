# llm fixtures

Real captures from mlx-serve on the Mac Pro (`:11234`, model `kota-local`, 2026-10-10,
flick-abcf). The wire parser tests in `openai.rs` read these:

- `mlx_serve_models.json`: `GET /v1/models`.
- `mlx_serve_stream.txt`: a streamed chat (`stream_options.include_usage`), no reasoning.
- `mlx_serve_reasoning.txt`: a streamed chat with `reasoning_content` deltas.
- `mlx_serve_error.json`: the 400 body for a malformed request.

Observed: every chunk carries `"usage":null`; a `: keepalive` comment arrives during prefill;
the final usage chunk adds `timings`; an unknown `model` is not an error (the server answers
with its one loaded model).

Synthetic scenario fixtures for the module tests (`testkit.rs`), shaped like the captures:

- `chat_stream.txt`: a fixed "Hello, world." reply.
- `models_mlx.json`: two models, one unloaded (the real server had only one).
- `models_ollama.json`: ollama was not running on any machine when the captures were made.
