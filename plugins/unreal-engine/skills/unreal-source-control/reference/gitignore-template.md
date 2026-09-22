# .gitignore template for a UE5 project

Copy the block below into `.gitignore` at the repository root (the folder containing the
`.uproject`). If the `.uproject` lives in a subfolder of the repo, the patterns without a
leading `/` still match; patterns with a leading `/` are anchored to the directory holding
the `.gitignore`, so put the file next to the `.uproject` or adjust the paths.

After adding it to an existing repo, untrack files that were committed before:
`git rm -r --cached Binaries Intermediate Saved DerivedDataCache` (then commit). This keeps
the files on disk and only removes them from the index. Check with `git status --ignored`.

```gitignore
# ---------------------------------------------------------------
# Unreal Engine generated folders (project and every plugin)
# ---------------------------------------------------------------
/Binaries/
/Intermediate/
/Saved/
/DerivedDataCache/
Plugins/**/Binaries/
Plugins/**/Intermediate/

# Staged/packaged output if someone archives inside the repo
/Packaged/
/Builds/
/ArchivedBuilds/

# Editor-generated per-user data that sometimes lands outside Saved/
*.tmp
*.bak

# ---------------------------------------------------------------
# IDE and project files (regenerate with GenerateProjectFiles)
# ---------------------------------------------------------------
.vs/
*.sln
*.suo
*.sdf
*.opensdf
*.VC.db
*.VC.opendb
*.vcxproj
*.vcxproj.filters
*.vcxproj.user
.idea/
*.DotSettings.user
.vscode/
*.code-workspace
*.xcodeproj/
*.xcworkspace/
compile_commands.json
.clangd/
.cache/

# ---------------------------------------------------------------
# Compiler/debugger intermediates that might appear outside
# Binaries/ and Intermediate/. (*.obj is deliberately NOT listed:
# it would also ignore Wavefront .obj meshes; compiled object files
# only appear under Intermediate/, which is already ignored.)
# ---------------------------------------------------------------
*.pch
*.gch
*.pdb
*.ilk
*.exp
*.ipdb
*.iobj

# ---------------------------------------------------------------
# Cooked / packaged content
# ---------------------------------------------------------------
*.pak
*.ucas
*.utoc

# ---------------------------------------------------------------
# OS noise
# ---------------------------------------------------------------
.DS_Store
Thumbs.db
Desktop.ini
```

## Notes on choices

- `Build/` is **not** ignored: it holds platform resources you need to package (Windows
  icon, Android manifest additions, iOS resources). If packaging writes generated files
  under `Build/` in your setup, add those specific paths.
- `Content/` is never ignored. `Content/Collections/` (shared collections) is committed;
  local collections live under `Saved/`.
- `.vsconfig` (written by the engine to tell Visual Studio which workloads to install) is
  harmless to commit and helps new machines; it is not ignored here.
- `Source/ThirdParty/**/*.lib|.a|.dll|.so` are real dependencies in many projects. They are
  not ignored above (only PCH/PDB intermediates are); store them with LFS (see the
  gitattributes template). If you commit ThirdParty `.pdb` files, remove `*.pdb`.
- If the team commits editor binaries for artists, remove `/Binaries/` and
  `Plugins/**/Binaries/` and add those files to LFS; otherwise never commit them.
- `*_BuiltData.uasset` is not ignored: whether to commit baked lighting data is a project
  policy (see SKILL.md section 1). To ignore it, add `*_BuiltData.uasset`.
- Relay and agent folders (`.claude/`, `.agents/`) are managed by Relay; follow the
  project's existing convention for whether they are committed.
