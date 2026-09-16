# Grid prototype

Tests whether a web-based window can scroll a very large library smoothly when thumbnails are served as plain URLs by the background process. This decides between one Tauri window shared by Mac and Windows and a native interface per platform. Throwaway code for learning, not the product.

## What it does

- A small Rust server (axum), standing in for the background process, generates 50,000 thumbnails and serves them at `/thumb/<id>.jpg` with long-lived cache headers, plus paged catalogue data at `/api/works`.
- A single HTML page renders a virtualised grid: only the visible rows (plus three rows of overscan) exist as elements, and tiles are recycled while scrolling.
- With `?autorun=1` the page benchmarks itself in three phases and posts the results back to the server:
  1. **Browse:** scroll at 1,500 px/s for 20 s, recording frame times and how many visible tiles are still blank.
  2. **Fling:** scroll through the entire library in 30 s (about 42,000 px/s).
  3. **Jump:** jump to 10 far-apart positions and time how long until every visible thumbnail is drawn.

The page runs in the browser engine under test. Safari uses WebKit, the engine behind Tauri's macOS window; Chrome and Edge use Chromium, the engine behind WebView2 in Tauri's Windows window.

## Running it

```sh
cargo run --release -p grid-spike            # 50,000 thumbnails on port 18480
cargo run --release -p grid-spike -- 100000  # other sizes
```

Open `http://127.0.0.1:18480/?autorun=1&engine=safari` (or `engine=chrome`, `engine=edge`) and keep the window in front for about a minute. Results print in the terminal and append to `target/grid-spike/results.jsonl`.

For meaningful frame rates: turn off Low Power Mode, put the window on a high-refresh display, and don't switch away during the run. Browsers slow down animation in background windows, in Low Power Mode, and on slow displays.

## Results

16 September 2026, MacBook Pro (Apple M1 Pro, 16 GB), 50,000 works, 6 columns. Raw data: [results/2026-09-16-macbook.json](results/2026-09-16-macbook.json).

| Engine and conditions | Browse | Fling (whole library in 30 s) | Blank tiles while browsing | Jump until filled (median / worst) | Peak renderer memory |
| --- | --- | --- | --- | --- | --- |
| **Chrome**, built-in 120 Hz display | **120 fps**, p99 9.4 ms | **120 fps**, p99 9.4 ms, worst 16.4 ms | 0% | 1 ms / 17 ms | 266 MB (JS heap 5 MB) |
| Safari, built-in display, Low Power Mode on | 30 fps (capped), p99 36 ms | 30 fps (capped), worst 47 ms | 0% | 1 ms / 33 ms | 295 MB |
| Safari, external display at 30 Hz, Low Power Mode on | 15 fps (capped) | 15 fps (capped) | 0% | under 1 ms / 64 ms | 206 MB |

### Findings

- **Chromium kept up with a 120 Hz display throughout,** even when flinging through all 50,000 works in 30 seconds: 0.03% of frames ran late and none took longer than 17 ms. Chromium is the engine behind WebView2, so this is encouraging for Windows, but a mid-range Windows laptop still needs its own run.
- **Virtualisation keeps the page tiny:** 42–72 tile elements at any time and a 5 MB JavaScript heap, regardless of library size.
- **Thumbnails served as URLs keep up:** no visible tile was blank while browsing, and after a jump to anywhere in the library the screen filled within one or two frames.
- **Safari couldn't be measured at full speed.** macOS Low Power Mode stayed on during the runs (`pmset` reported it on for both battery and AC power), which caps Safari at 30 fps; the external display ran at 30 Hz. Within those caps Safari dropped no frames. Rerun with Low Power Mode actually off to confirm WebKit reaches 120 Hz.
- **Browser memory is dominated by the engine, not the grid:** roughly 200–300 MB for the page's renderer process in both engines.

### Caveats

- Synthetic thumbnails averaged 3 KB; real scans measured about 16 KB. Decoding work per pixel is similar, but larger files mean more bytes read and parsed.
- Loopback HTTP on a fast Mac. The Windows test matters most for the decision.
- The grid is plain JavaScript, not the Svelte `WorksGrid` from the previous app, and has no selection, drag, or captions beyond a line of text.
- This measures the browser engines directly, not a built Tauri app. Tauri adds its own shell but loads images the same way when they come from local HTTP URLs.

## Dependencies

anyhow, serde, serde_json, tokio, axum, and image (JPEG only), all within the [dependency policy](../../docs/dependency-policy.md).
