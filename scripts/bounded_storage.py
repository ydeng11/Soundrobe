"""Mock helper output through the maintained native storage boundary."""
import os
import pathlib
import subprocess


def _scoped():
    if os.environ.get("SOUNDROBE_NATIVE_MANAGED") == "1" and os.environ.get("SOUNDROBE_STORAGE_REQUIRED") != "1":
        raise ValueError("managed output has no storage authority")
    return os.environ.get("SOUNDROBE_STORAGE_REQUIRED") == "1"


def _sink(path, action, content=b""):
    if len(content) > 8388608:
        raise ValueError("storage helper input ceiling")
    keys = ["SOUNDROBE_NATIVE_OUTPUT", "SOUNDROBE_NATIVE_GENERATION", "SOUNDROBE_STORAGE_EXECUTABLE"]
    env = {key: os.environ[key] for key in keys}
    env.update(SOUNDROBE_STORAGE_REQUIRED="1", SOUNDROBE_STORAGE_TARGET=str(pathlib.Path(path).absolute()),
               SOUNDROBE_STORAGE_ACTION=action)
    subprocess.run([env["SOUNDROBE_STORAGE_EXECUTABLE"], "--exact",
                    "infra::mock_run_authority::storage::tests::storage_sink_process", "--ignored", "--nocapture"],
                   input=content, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)


def write_text(path, content, exclusive=False):
    if _scoped():
        _sink(path, "create" if exclusive else "overwrite", content.encode("utf-8"))
    else:
        with pathlib.Path(path).open("x" if exclusive else "w", encoding="utf-8") as output:
            output.write(content)


def mkdir(path):
    if _scoped():
        _sink(path, "mkdir_all")
    else:
        pathlib.Path(path).mkdir(parents=True, exist_ok=True)
