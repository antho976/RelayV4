# .gitattributes template for a UE5 project (Git LFS + locking)

Copy the block below into `.gitattributes` at the repository root and commit it **before**
committing any assets. Requires Git LFS (`git lfs install`) on every machine that clones
the repo.

- `filter=lfs diff=lfs merge=lfs -text` stores the file in LFS and never treats it as text.
- `lockable` makes Git LFS check the file out read-only until you `git lfs lock` it. It is
  set on Unreal assets and on unmergeable authoring files (layered PSDs, DCC scenes).
  Remove `lockable` from a line if the team does not want locking for that type.
- Text rules normalize line endings in the repo (LF) while letting each OS check out its
  native endings; `.bat`/`.cmd` stay CRLF and shell scripts stay LF on every OS.

```gitattributes
# ---------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------
* text=auto

# ---------------------------------------------------------------
# Text sources and config
# ---------------------------------------------------------------
*.cpp       text diff=cpp
*.c         text diff=cpp
*.cc        text diff=cpp
*.h         text diff=cpp
*.hpp       text diff=cpp
*.inl       text diff=cpp
*.cs        text diff=csharp
*.py        text diff=python
*.ini       text
*.uproject  text
*.uplugin   text
*.json      text
*.xml       text
*.yml       text
*.yaml      text
*.md        text
*.txt       text
*.csv       text
*.usf       text
*.ush       text
*.hlsl      text
*.glsl      text
*.usda      text
*.bat       text eol=crlf
*.cmd       text eol=crlf
*.ps1       text eol=crlf
*.sh        text eol=lf
*.command   text eol=lf

# ---------------------------------------------------------------
# Unreal assets: LFS + locking (binary, never mergeable)
# ---------------------------------------------------------------
*.uasset    filter=lfs diff=lfs merge=lfs -text lockable
*.umap      filter=lfs diff=lfs merge=lfs -text lockable
*.upk       filter=lfs diff=lfs merge=lfs -text lockable
*.udk       filter=lfs diff=lfs merge=lfs -text lockable

# ---------------------------------------------------------------
# Authoring / DCC source files: LFS + locking
# ---------------------------------------------------------------
*.psd       filter=lfs diff=lfs merge=lfs -text lockable
*.psb       filter=lfs diff=lfs merge=lfs -text lockable
*.kra       filter=lfs diff=lfs merge=lfs -text lockable
*.xcf       filter=lfs diff=lfs merge=lfs -text lockable
*.blend     filter=lfs diff=lfs merge=lfs -text lockable
*.max       filter=lfs diff=lfs merge=lfs -text lockable
*.ma        filter=lfs diff=lfs merge=lfs -text lockable
*.mb        filter=lfs diff=lfs merge=lfs -text lockable
*.c4d       filter=lfs diff=lfs merge=lfs -text lockable
*.hip       filter=lfs diff=lfs merge=lfs -text lockable
*.hiplc     filter=lfs diff=lfs merge=lfs -text lockable
*.ztl       filter=lfs diff=lfs merge=lfs -text lockable
*.zpr       filter=lfs diff=lfs merge=lfs -text lockable
*.spp       filter=lfs diff=lfs merge=lfs -text lockable
*.sbs       filter=lfs diff=lfs merge=lfs -text lockable
*.sbsar     filter=lfs diff=lfs merge=lfs -text lockable

# ---------------------------------------------------------------
# Exported meshes / scenes: LFS (re-exported, so no locking by default)
# ---------------------------------------------------------------
*.fbx       filter=lfs diff=lfs merge=lfs -text
*.obj       filter=lfs diff=lfs merge=lfs -text
*.gltf      filter=lfs diff=lfs merge=lfs -text
*.glb       filter=lfs diff=lfs merge=lfs -text
*.abc       filter=lfs diff=lfs merge=lfs -text
*.usd       filter=lfs diff=lfs merge=lfs -text
*.usdc      filter=lfs diff=lfs merge=lfs -text
*.usdz      filter=lfs diff=lfs merge=lfs -text
*.3ds       filter=lfs diff=lfs merge=lfs -text
*.dae       filter=lfs diff=lfs merge=lfs -text

# ---------------------------------------------------------------
# Images and textures
# ---------------------------------------------------------------
*.png       filter=lfs diff=lfs merge=lfs -text
*.jpg       filter=lfs diff=lfs merge=lfs -text
*.jpeg      filter=lfs diff=lfs merge=lfs -text
*.tga       filter=lfs diff=lfs merge=lfs -text
*.tif       filter=lfs diff=lfs merge=lfs -text
*.tiff      filter=lfs diff=lfs merge=lfs -text
*.bmp       filter=lfs diff=lfs merge=lfs -text
*.gif       filter=lfs diff=lfs merge=lfs -text
*.exr       filter=lfs diff=lfs merge=lfs -text
*.hdr       filter=lfs diff=lfs merge=lfs -text
*.dds       filter=lfs diff=lfs merge=lfs -text
*.ico       filter=lfs diff=lfs merge=lfs -text
*.icns      filter=lfs diff=lfs merge=lfs -text
*.webp      filter=lfs diff=lfs merge=lfs -text

# ---------------------------------------------------------------
# Audio and video
# ---------------------------------------------------------------
*.wav       filter=lfs diff=lfs merge=lfs -text
*.mp3       filter=lfs diff=lfs merge=lfs -text
*.ogg       filter=lfs diff=lfs merge=lfs -text
*.flac      filter=lfs diff=lfs merge=lfs -text
*.aif       filter=lfs diff=lfs merge=lfs -text
*.aiff      filter=lfs diff=lfs merge=lfs -text
*.bnk       filter=lfs diff=lfs merge=lfs -text
*.mp4       filter=lfs diff=lfs merge=lfs -text
*.mov       filter=lfs diff=lfs merge=lfs -text
*.avi       filter=lfs diff=lfs merge=lfs -text
*.webm      filter=lfs diff=lfs merge=lfs -text
*.bk2       filter=lfs diff=lfs merge=lfs -text

# ---------------------------------------------------------------
# Fonts
# ---------------------------------------------------------------
*.ttf       filter=lfs diff=lfs merge=lfs -text
*.otf       filter=lfs diff=lfs merge=lfs -text

# ---------------------------------------------------------------
# Third-party / prebuilt binaries (only if committed deliberately)
# ---------------------------------------------------------------
*.dll       filter=lfs diff=lfs merge=lfs -text
*.so        filter=lfs diff=lfs merge=lfs -text
*.dylib     filter=lfs diff=lfs merge=lfs -text
*.lib       filter=lfs diff=lfs merge=lfs -text
*.a         filter=lfs diff=lfs merge=lfs -text
*.exe       filter=lfs diff=lfs merge=lfs -text
*.zip       filter=lfs diff=lfs merge=lfs -text
*.7z        filter=lfs diff=lfs merge=lfs -text
*.pdf       filter=lfs diff=lfs merge=lfs -text
```

## After committing it

```
git add .gitattributes && git commit -m "Track Unreal assets with Git LFS"
git lfs ls-files                              # lists LFS-tracked files once assets are added
git check-attr -a Content/Maps/Main.umap      # expect filter: lfs, lockable: set
git config lfs.<remote-lfs-url>.locksverify true   # refuse pushes that touch others' locks
```

## Notes

- `.gitattributes` only affects files added after it is committed. Files already in history
  as normal blobs stay that way until `git lfs migrate import` rewrites history (a human
  decision; every clone must re-clone).
- `*.obj` here means Wavefront meshes. Compiled `.obj` object files only appear under
  `Intermediate/`, which the gitignore template ignores as a folder (the template does not
  ignore `*.obj` globally, precisely so meshes are not lost).
- `*.usda` is text USD; binary USD (`.usd`, `.usdc`, `.usdz`) goes to LFS. If your `.usd`
  files are ASCII, move `*.usd` to the text section.
- Do not add `lockable` to text types: locking is only for files that cannot be merged.
- With `lockable`, tools that write files in place (the editor, DCC exporters) fail on
  read-only files until the lock is taken. That is intended: take the lock first.
