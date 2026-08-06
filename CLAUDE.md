# HD2 Repatcher

After making changes to this repo, run `bun run check:agent` (typecheck +
`cargo check` + biome format, run concurrently) before considering the work
done.

This repo uses [Jujutsu](https://jj-vcs.github.io/jj/) (`jj`), not a bare git
working copy — there is no `.git` checkout, only `.jj`. If changes were made
and you're running in auto mode, commit them with `jj commit` (or
`jj describe` on the current working-copy commit) once `check:agent` passes.
