# Storm Uploader — Claude Context

## Releasing a new version

Releases are driven by the `bump-version.yml` workflow, which bumps the version, tags, pushes, and chains into `release.yml`. Everything runs server-side on GitHub Actions — no local push needed.

### Trigger a bump

```sh
gh workflow run bump-version.yml -f bump=patch   # or minor / major
```

### Watch it through to completion

The bump itself finishes in ~15s, but the chained release build takes several minutes. Both need to be watched.

```sh
# 1. Grab the run ID for the bump
gh run list --workflow=bump-version.yml --limit 1

# 2. Watch it
gh run watch <bump-run-id>

# 3. Once the bump succeeds, the release workflow is already running — grab its ID and watch it too
gh run list --workflow=release.yml --limit 1
gh run watch <release-run-id>
```

For long-running watches, prefer `run_in_background: true` on the Bash tool call rather than blocking the session.

### Notes

- The `bump-version.yml` workflow pushes the version commit and tag to `main` itself. If there are unpushed local commits, rebase them onto the new `main` afterward (`git pull --rebase origin main`) rather than racing the workflow.
- Release artifacts land on the GitHub Releases page once `release.yml` finishes.
