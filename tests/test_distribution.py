"""Tests for release packaging artifacts."""

from pathlib import Path

import tomllib


def test_pyproject_metadata_is_release_ready():
    """Package metadata exposes scripts, URLs, and build tooling."""
    data = tomllib.loads(Path("pyproject.toml").read_text(encoding="utf-8"))

    assert data["project"]["scripts"]["auto-tag"] == "auto_tagger.cli:main"
    assert "build>=1.0" in data["project"]["optional-dependencies"]["dev"]
    assert "yourusername" not in data["project"]["urls"]["Repository"]


def test_homebrew_formula_template_exists():
    """Homebrew formula template contains install and smoke-test blocks."""
    formula = Path("packaging/homebrew/auto-tagger.rb").read_text(encoding="utf-8")

    assert "class AutoTagger < Formula" in formula
    assert "virtualenv_install_with_resources" in formula
    assert "auto-tag --version" in formula


def test_release_checklist_documents_credentialed_steps():
    """Release checklist documents build and manual upload commands."""
    checklist = Path("docs/release-checklist.md").read_text(encoding="utf-8")

    assert "python -m build" in checklist
    assert "TestPyPI" in checklist
    assert "Do not commit API tokens" in checklist
