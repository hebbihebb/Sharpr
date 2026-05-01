# Test Assets

This directory contains deterministic images used for `cargo test` and CI.

- `test.jxl`: A sample JXL file used for testing the JXL preview and thumbnail decoding pathways. This tests the JXL fast-paths and fallback logic. Since JXL encoding tooling is not available directly within the Rust ecosystem without external libraries, we provide this checked-in asset. If absent, the tests will skip gracefully with a clear message.
- `test.png`: A reasonably large RGBA PNG file used for PNG fast-path testing.

## Regenerating

- If you need to update `test.jxl`, simply encode a new JXL image and place it here as `test.jxl`. The JXL image should ideally contain a preview frame if you want to test embedded preview logic.
- If you need to update `test.png`, replace it with any valid PNG file. Some tests may dynamically generate smaller PNGs during runtime, but `test.png` is available for integration-style path loading tests.

These assets ensure `cargo test` avoids downloading external images from the network and does not depend on a local developer's photo library.
