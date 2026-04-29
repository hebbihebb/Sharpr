# TODO

## ComfyUI Upscaling Follow-Ups

- Handle ComfyUI jobs that complete without output images. Right now Sharpr polls `/history/{prompt_id}` until timeout if ComfyUI returns an empty cached result for the same image/model combination.
- Investigate how to avoid ComfyUI cache collisions for repeated runs of the same image with the same model name. Current observed behavior is that rerunning the same image can produce no outputs until ComfyUI is restarted.
- Improve the compare/save flow messaging so it is clear when the exported file was already written with a suffixed name in the `exported/` folder.
- Fix the misleading ComfyUI scale UI. The current backend always executes a remote `4x` workflow and only applies smaller requested scales locally afterward.

## ComfyUI Workflow Flexibility

- Stop hardcoding the workflow around fixed node IDs and fixed model-name patching only.
- Evaluate a safer workflow contract so Sharpr can support changed ComfyUI pipelines without code edits every time.
- Consider discovering the active workflow shape or a configurable workflow schema instead of assuming:
  - node `1.inputs.image`
  - node `3.inputs.model_name`
  - first output image in history
- Investigate whether Sharpr should store a user-provided workflow preset path or JSON blob instead of shipping only one fixed `comfy_preset.json`.
- Support workflows that emit output images from different nodes, subfolders, or multiple outputs instead of downloading only by bare filename.
- Revisit model selection so Sharpr is not permanently coupled to the two hardcoded names `RealESRGAN_x4plus.pth` and `RealESRGAN_x4plus_anime.pth`.

## Current Server Notes

- Preferred LAN URL for the current setup: `http://192.168.8.104:8188`
- Tailscale fallback URL: `http://100.121.114.22:8188`
- Confirmed server-side behavior:
  - `GET /system_stats`
  - `POST /upload/image`
  - `POST /prompt`
  - `GET /history/{prompt_id}`
  - `GET /view?filename=...`
- Current ComfyUI model mapping on the server:
  - `RealESRGAN_x4plus.pth` currently resolves via symlink to `4x-UltraSharpV2.pth`
  - `RealESRGAN_x4plus_anime.pth` is the original anime model
