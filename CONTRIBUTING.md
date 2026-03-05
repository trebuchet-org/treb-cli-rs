# Contributing

## Working with the treb-sol Submodule

This project includes `lib/treb-sol` as a git submodule (with its own nested submodules).

### Cloning the Repository

```sh
git clone --recurse-submodules <repo-url>
```

If you already cloned without `--recurse-submodules`:

```sh
git submodule update --init --recursive
```

### Updating the Submodule to a New Commit

```sh
cd lib/treb-sol
git fetch && git checkout <commit-or-tag>
cd ../..
git add lib/treb-sol
git commit -m "chore: update treb-sol submodule to <commit-or-tag>"
```

### CI Configuration

CI checkout steps must enable recursive submodules. For GitHub Actions:

```yaml
- uses: actions/checkout@v4
  with:
    submodules: recursive
```
