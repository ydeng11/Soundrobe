# Release Checklist

## Local Build

```bash
python -m build
python -m pip install --force-reinstall dist/auto_tagger-*.whl
auto-tag --version
```

## TestPyPI

```bash
python -m twine upload --repository testpypi dist/*
python -m pip install --index-url https://test.pypi.org/simple/ auto-tagger
```

## PyPI

```bash
python -m twine upload dist/*
```

Do not commit API tokens, PyPI tokens, Homebrew credentials, or `.pypirc`.

## Homebrew

1. Update `packaging/homebrew/auto-tagger.rb` with the final PyPI source URL.
2. Replace `UPDATE_AFTER_PYPI_RELEASE` with the source archive SHA256.
3. Run `brew audit --strict --online auto-tagger`.
4. Run `brew test auto-tagger`.
