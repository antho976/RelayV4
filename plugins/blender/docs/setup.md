# Setting up Blender for the Relay plugin

1. **Install Blender** (4.0 or newer; the tools are tested on 5.2 LTS, and LTS releases are the safest). The official download from
   blender.org bundles everything the tools need.
2. **Make it findable.** Relay looks for `blender` on `PATH`, then
   `/Applications/Blender.app/Contents/MacOS/Blender`, `/snap/bin/blender`, `/usr/local/bin/blender`
   and the newest `C:/Program Files/Blender Foundation/Blender */blender.exe`. Otherwise set
   **`BLENDER_BIN`** to the executable in the environment Relay's engine is started from.
   `blender_info` with no file reports which Blender it found.
3. **Linux distribution packages** can lack pieces the official build bundles. Blender needs
   `libEGL` even in background mode, and the FBX exporter needs `numpy`
   (on Ubuntu: `libegl1`, `libgl1-mesa-dri`, `python3-numpy`). Cycles in some distribution builds
   has no denoiser; the render tool turns denoising off.

The tools start Blender with factory settings, so your own add-ons and preferences are not
loaded. Bundled add-ons (Rigify, for example) can be enabled per call with `blender_python`'s
`addons`.

`.blend` files are binary. Keep them in Git LFS (the `unreal-source-control` skill has a
`.gitattributes` template that covers them) and do not let two agents edit the same file.
