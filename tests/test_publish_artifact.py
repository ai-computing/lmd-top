import json
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

path = Path(__file__).resolve().parents[1] / "assets/recipes/publish-artifact.py"
spec = importlib.util.spec_from_file_location("publish_artifact", path)
publisher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publisher)


class PublishArtifactTests(unittest.TestCase):
    def test_complete_build_published_without_overwriting(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source, destination = root / "source", root / "store/build"
            source.mkdir()
            (source / "weights").write_text("first")
            publisher.publish(source, destination)
            self.assertEqual((destination / "weights").read_text(), "first")
            self.assertTrue((destination / "lmd-build.json").exists())
            (source / "weights").write_text("replacement")
            with self.assertRaises(FileExistsError):
                publisher.publish(source, destination)
            self.assertEqual((destination / "weights").read_text(), "first")
            self.assertEqual(list(destination.parent.iterdir()), [destination])

    def test_copy_failure_never_exposes_partial_build(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source, destination = root / "source", root / "store/build"
            source.mkdir()
            (source / "weights").write_text("data")
            with patch.object(publisher.shutil, "copytree", side_effect=OSError("disk full")):
                with self.assertRaises(OSError):
                    publisher.publish(source, destination)
            self.assertFalse(destination.exists())
            self.assertEqual(list(destination.parent.iterdir()), [])

    def test_concurrent_publisher_lock_is_not_removed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            lock = root / ".lmd-publish-build.lock"
            lock.mkdir()
            with self.assertRaises(FileExistsError):
                publisher.publish(root / "source", root / "build")
            self.assertTrue(lock.exists())


class BuildMetadataTests(unittest.TestCase):
    def test_provenance_changes_with_options_and_toolchain_without_exposing_environment(self):
        with tempfile.TemporaryDirectory() as tmp, patch.dict(publisher.os.environ, {
            "MODEL_ID": "org/model", "LMD_BUILD_VENDOR": "rbln", "LMD_BUILD_IMAGE": "compiler:tag",
            "LMD_BUILD_OPTIONS": '{"tp":"2","HF_TOKEN":"do-not-record"}', "HF_TOKEN": "secret-env",
        }, clear=True), patch.object(publisher.importlib.metadata, "version", return_value="1.0"):
            root = Path(tmp)
            (root / "config.json").write_text(json.dumps({"_commit_hash": "a" * 40}))
            first = publisher.build_metadata(root, root / "build", {"rbln_num_devices": 2})
            same = publisher.build_metadata(root, root / "build", {"rbln_num_devices": 2})
            changed = publisher.build_metadata(root, root / "build", {"rbln_num_devices": 4})
            self.assertEqual(first["provenance_sha256"], same["provenance_sha256"])
            self.assertNotEqual(first["provenance_sha256"], changed["provenance_sha256"])
            with patch.object(publisher.importlib.metadata, "version", return_value="2.0"):
                tools_changed = publisher.build_metadata(root, root / "build", {"rbln_num_devices": 2})
            self.assertNotEqual(first["provenance_sha256"], tools_changed["provenance_sha256"])
            self.assertEqual(first["provenance"]["resolved_revision"], "a" * 40)
            self.assertEqual(first["compatibility"], "unknown")
            self.assertEqual(len(first["artifact_content_sha256"]), 64)
            self.assertNotIn("do-not-record", json.dumps(first))
            self.assertNotIn("secret-env", json.dumps(first))

    def test_unknown_revision_and_actual_furiosa_command_options_are_explicit(self):
        with tempfile.TemporaryDirectory() as tmp, patch.dict(publisher.os.environ, {
            "LMD_BUILD_VENDOR":"furiosa", "TP":"4", "PP":"2", "MAX_LEN":"8192",
        }, clear=True):
            metadata = publisher.build_metadata(Path(tmp), "build")
            self.assertIsNone(metadata["provenance"]["resolved_revision"])
            self.assertEqual(metadata["provenance"]["effective_options"], {"tp":4,"pp":2,"max_model_len":8192,"concurrency":8,"optim_level":"O3"})

    def test_custom_furiosa_workers_and_bucket_level_are_recorded(self):
        with tempfile.TemporaryDirectory() as tmp, patch.dict(publisher.os.environ, {
            "LMD_BUILD_VENDOR": "furiosa", "TP": "8", "PP": "1", "MAX_LEN": "4096",
            "FXB_CONCURRENCY": "3", "FXB_OPTIM_LEVEL": "O0",
            "LMD_BUILD_OPTIONS": '{"concurrency":"3","optim":"O0"}',
        }, clear=True):
            provenance = publisher.build_metadata(Path(tmp), "build")["provenance"]
            self.assertEqual(provenance["effective_options"]["concurrency"], 3)
            self.assertEqual(provenance["effective_options"]["optim_level"], "O0")
            self.assertEqual(provenance["requested_options"], {"concurrency":"3", "optim":"O0"})

    def test_content_manifest_tracks_bytes_and_paths_not_metadata_or_time(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "weights.bin").write_bytes(b"abc")
            first, digest = publisher.artifact_manifest(root)
            self.assertEqual(first[0]["size_bytes"], 3)
            (root / "lmd-build.json").write_text("metadata changed")
            self.assertEqual(publisher.artifact_manifest(root)[1], digest)
            (root / "weights.bin").write_bytes(b"abd")
            self.assertNotEqual(publisher.artifact_manifest(root)[1], digest)
            (root / "weights.bin").write_bytes(b"abc")
            (root / "weights.bin").rename(root / "renamed.bin")
            self.assertNotEqual(publisher.artifact_manifest(root)[1], digest)

    def test_published_content_verification_rejects_tampering(self):
        with tempfile.TemporaryDirectory() as tmp, patch.dict(publisher.os.environ, {}, clear=True):
            root = Path(tmp)
            source = root / "source"
            source.mkdir()
            (source / "model.bin").write_bytes(b"complete")
            destination = root / "published"
            publisher.publish(source, destination)
            self.assertEqual(len(publisher.verify_artifact(destination)), 64)
            (destination / "model.bin").write_bytes(b"corrupt")
            with self.assertRaisesRegex(ValueError, "verification failed"):
                publisher.verify_artifact(destination)

    def test_directory_links_cannot_escape_content_verification(self):
        with tempfile.TemporaryDirectory() as tmp, patch.dict(publisher.os.environ, {}, clear=True):
            root = Path(tmp)
            source = root / "source"
            source.mkdir()
            (source / "model.bin").write_bytes(b"complete")
            destination = root / "published"
            publisher.publish(source, destination)
            (destination / "extra").symlink_to(source, target_is_directory=True)
            with self.assertRaisesRegex(ValueError, "artifact link"):
                publisher.verify_artifact(destination)
            (source / "external").symlink_to(destination, target_is_directory=True)
            with self.assertRaisesRegex(ValueError, "artifact link"):
                publisher.publish(source, root / "rejected")
            self.assertFalse((root / "rejected").exists())
            self.assertFalse(list(root.glob(".lmd-*")))

    def test_walk_errors_are_not_hidden(self):
        def unreadable(*args, **kwargs):
            kwargs["onerror"](PermissionError("unreadable subtree"))
        with tempfile.TemporaryDirectory() as tmp, patch.object(publisher.os, "walk", side_effect=unreadable):
            with self.assertRaises(PermissionError):
                publisher.artifact_manifest(tmp)

    def test_invalid_metadata_does_not_publish_a_build(self):
        with tempfile.TemporaryDirectory() as tmp, patch.dict(publisher.os.environ, {"LMD_BUILD_OPTIONS":"not json"}):
            root=Path(tmp); source=root/"source"; source.mkdir(); (source/"artifact").write_text("complete")
            with self.assertRaises(ValueError): publisher.publish(source,root/"store/build")
            self.assertFalse((root/"store/build").exists())
            self.assertEqual(list((root/"store").iterdir()),[])
