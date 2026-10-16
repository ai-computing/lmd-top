#!/usr/bin/env python3
"""Publish a complete build on the store filesystem without merging or overwriting.

Staging and locks are hidden siblings, excluded by the discovery CronJob's * glob.
A killed publisher may leave a hidden lock/staging directory for explicit recovery;
readers continue seeing either no build or the previously published complete build.
"""
import hashlib
import importlib.metadata
import platform
import re
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import shutil
import stat
import sys
import tempfile


PACKAGES = ("optimum-rbln", "rebel-compiler", "furiosa-llm", "furiosa-compiler", "transformers", "torch")

def artifact_manifest(source):
    """Hash files in bounded chunks; paths are relative POSIX names, sorted canonically.

    The root metadata file is excluded to avoid a recursive checksum. This observes
    bytes in the private staging copy, not mutable source paths or file timestamps.
    """
    source = Path(source)
    if not source.is_dir() or source.is_symlink():
        raise ValueError("artifact root must be a real directory")
    entries = []
    # os.walk with onerror avoids treating unreadable subtrees as an empty tree.
    def failed(error):
        raise error
    paths = []
    for directory, directories, files in os.walk(source, followlinks=False, onerror=failed):
        paths.extend(Path(directory) / name for name in directories + files)
    for path in sorted(paths):
        relative = path.relative_to(source).as_posix()
        mode = path.lstat().st_mode
        if stat.S_ISLNK(mode):
            raise ValueError(f"unsupported artifact link: {relative}")
        if stat.S_ISDIR(mode):
            continue
        if not stat.S_ISREG(mode):
            raise ValueError(f"unsupported artifact entry: {relative}")
        if relative == "lmd-build.json":
            continue
        digest = hashlib.sha256()
        size = 0
        with path.open("rb") as stream:
            while chunk := stream.read(1024 * 1024):
                size += len(chunk)
                digest.update(chunk)
        entries.append({"path": relative, "size_bytes": size, "sha256": digest.hexdigest()})
    canonical = json.dumps(entries, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode("utf-8")
    return entries, hashlib.sha256(canonical).hexdigest()


def verify_artifact(directory):
    directory = Path(directory)
    metadata_path = directory / "lmd-build.json"
    if metadata_path.is_symlink() or not stat.S_ISREG(metadata_path.lstat().st_mode):
        raise ValueError("build metadata must be a regular file")
    metadata = json.loads(metadata_path.read_text())
    manifest = metadata.get("artifact_manifest", {})
    if manifest.get("algorithm") != "sha256-canonical-file-list-v1":
        raise ValueError("artifact has no supported content manifest")
    files, digest = artifact_manifest(directory)
    if files != manifest.get("files") or digest != metadata.get("artifact_content_sha256"):
        raise ValueError("artifact content verification failed")
    return digest


def build_metadata(source, destination, effective_options=None):
    """Record observed facts only. This fingerprint is NOT a hash of artifact bytes."""
    source = Path(source)
    packages = {}
    for package in PACKAGES:
        try:
            packages[package] = importlib.metadata.version(package)
        except importlib.metadata.PackageNotFoundError:
            packages[package] = None
    revision = None
    config = source / "config.json"
    if config.is_file():
        try:
            candidate = json.loads(config.read_text()).get("_commit_hash")
            if isinstance(candidate, str) and re.fullmatch(r"[0-9a-fA-F]{40}", candidate):
                revision = candidate.lower()
        except (ValueError, OSError):
            pass
    requested = json.loads(os.environ.get("LMD_BUILD_OPTIONS", "{}"))
    if not isinstance(requested, dict):
        raise ValueError("LMD_BUILD_OPTIONS must be a JSON object")
    allowed = {"tp", "pp", "max-len", "batch", "attn", "kvpart", "quant", "npu", "chunk", "block", "devices", "concurrency", "optim"}
    requested = {key: value for key, value in requested.items() if key in allowed}
    vendor = os.environ.get("LMD_BUILD_VENDOR")
    if effective_options is None and vendor == "furiosa":
        effective_options = {"tp": int(os.environ["TP"]), "pp": int(os.environ["PP"]),
                             "max_model_len": int(os.environ["MAX_LEN"]), "concurrency": int(os.environ.get("FXB_CONCURRENCY", "8")),
                             "optim_level": os.environ.get("FXB_OPTIM_LEVEL", "O3")}
    recipes = {}
    for name in ("compile.py", "compile.sh", "publish-artifact.py"):
        path = Path(__file__).parent / name
        if path.is_file():
            recipes[name] = hashlib.sha256(path.read_bytes()).hexdigest()
    identity = {
        "model": os.environ.get("MODEL_ID"), "resolved_revision": revision,
        "vendor": vendor, "requested_image": os.environ.get("LMD_BUILD_IMAGE"),
        "requested_options": requested, "effective_options": effective_options,
        "packages": packages, "recipes_sha256": recipes,
        "python": platform.python_version(), "machine": platform.machine(),
    }
    fingerprint = hashlib.sha256(json.dumps(identity, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()).hexdigest()
    files, content_hash = artifact_manifest(source)
    return {
        "schema": "lmd-top/build/v2", "model": identity["model"],
        "execution_uid": os.environ.get("LMD_BUILD_UID", ""),
        "execution_uid_kind": "Pod", "target": Path(destination).name,
        "created_at": datetime.now(timezone.utc).isoformat(),
        "provenance": identity, "provenance_sha256": fingerprint,
        "artifact_content_sha256": content_hash,
        "artifact_manifest": {"algorithm": "sha256-canonical-file-list-v1", "files": files},
        "observed_image_digest": None,
        "compatibility": "unknown", "driver_version": None, "firmware_version": None,
        "limitations": ["fingerprint covers recorded provenance, not artifact contents",
                        "arbitrary environment overrides are not captured",
                        "runtime compatibility requires independent evidence"],
    }


def publish(source, destination, effective_options=None):
    source, destination = Path(source), Path(destination)
    parent = destination.parent
    parent.mkdir(parents=True, exist_ok=True)
    lock = parent / (".lmd-publish-" + destination.name + ".lock")
    # Atomic mkdir serializes cooperating publishers, also on the shared SMB mount.
    lock.mkdir()
    stage = None
    try:
        if destination.exists() or destination.is_symlink():
            raise FileExistsError(f"build already published: {destination}; move/delete explicitly before rebuilding")
        if not source.is_dir() or not any(source.iterdir()):
            raise ValueError("compiler produced no artifact")
        stage = Path(tempfile.mkdtemp(prefix=".lmd-staging-", dir=parent))
        shutil.copytree(source, stage, dirs_exist_ok=True, symlinks=True)
        metadata = build_metadata(stage, destination, effective_options)
        (stage / "lmd-build.json").write_text(json.dumps(metadata, sort_keys=True, allow_nan=False) + "\n")
        # Same-filesystem directory rename publishes all files together. Never fall back
        # to a copy into destination if this filesystem cannot perform the rename.
        stage.rename(destination)
        stage = None
    finally:
        if stage is not None:
            shutil.rmtree(stage, ignore_errors=True)
        lock.rmdir()


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--verify":
        print("CONTENT_VERIFIED", verify_artifact(sys.argv[2]), flush=True)
    else:
        publish(sys.argv[1], sys.argv[2])
        print("COMPILE_DONE", sys.argv[2], flush=True)
