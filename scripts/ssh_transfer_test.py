"""Transfer metadata, integrity, and overwrite boundaries; no external ports or paid services."""
import hashlib
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src-tauri/src/media"))
import ssh_transfer as transfer
import ssh_worker as worker


class DirectTransferTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.source = self.root / str(uuid.uuid4())
        self.destination = self.root / str(uuid.uuid4())
        (self.source / "segments").mkdir(parents=True)
        self.destination.mkdir()
        self.data = b"synthetic segment fixture"
        (self.source / "segments/seg_100000.mp4").write_bytes(self.data)
        self.ticket = {"localJob":self.source.name,"files":[{"name":"seg_100000.mp4","bytes":len(self.data),"sha256":hashlib.sha256(self.data).hexdigest()}]}
        self.patch = patch.object(worker, "root_directory", return_value=self.root)
        self.patch.start()

    def tearDown(self):
        self.patch.stop()
        self.tmp.cleanup()

    def test_copy_within_primary_keeps_source_until_verification(self):
        receipt = transfer.receive(self.destination, self.ticket)
        self.assertEqual(receipt["files"], self.ticket["files"])
        self.assertEqual((self.destination / "seg_100000.mp4").read_bytes(),self.data)
        self.assertTrue((self.source / "segments/seg_100000.mp4").exists())

    def test_tampered_segment_never_becomes_a_completed_file(self):
        self.ticket["files"][0]["sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError,"Checksum"):
            transfer.receive(self.destination,self.ticket)
        self.assertEqual(list(self.destination.iterdir()),[])
        self.assertTrue((self.source / "segments/seg_100000.mp4").exists())

    def test_destination_is_never_overwritten(self):
        (self.destination / "seg_100000.mp4").write_bytes(b"existing")
        with self.assertRaisesRegex(ValueError,"overwrite"):
            transfer.receive(self.destination,self.ticket)
        self.assertEqual((self.destination / "seg_100000.mp4").read_bytes(),b"existing")

    def test_existing_partial_from_another_attempt_is_preserved(self):
        partial = self.destination / "seg_100000.mp4.part"
        partial.write_bytes(b"other attempt")
        with self.assertRaises(FileExistsError):
            transfer.receive(self.destination,self.ticket)
        self.assertEqual(partial.read_bytes(),b"other attempt")

    def test_names_hashes_sizes_and_https_are_validated(self):
        for invalid in (None,{}, {"files":[None]}, {"files":[{"name":"../private"}]}, {"files":[dict(self.ticket["files"][0],bytes=-1)]}, {"files":self.ticket["files"]*2}):
            with self.assertRaises(ValueError):
                transfer.files(invalid)
        for url in ("http://example.test", "https://user:secret@example.test", "https://example.test/?token=unsafe"):
            with self.assertRaises(ValueError):
                transfer.receive(self.destination,{"url":url,"files":self.ticket["files"]})


if __name__ == "__main__":
    unittest.main()
