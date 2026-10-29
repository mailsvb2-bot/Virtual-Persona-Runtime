"""Shared worker/browser ICE configuration and fail-closed validation."""
import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))

from rt1_aiortc_backend import parse_ice_servers


class IceConfigTests(unittest.TestCase):
    def test_default_local_only(self):
        self.assertEqual(parse_ice_servers(None), [])

    def test_stun_and_authenticated_turn(self):
        data = [
            {"urls": ["stun:stun.example.org:3478"]},
            {
                "urls": ["turns:relay.example.org:5349?transport=tcp"],
                "username": "ephemeral-user",
                "credential": "ephemeral-secret",
            },
        ]
        self.assertEqual(parse_ice_servers(json.dumps(data)), [
            {"urls": data[0]["urls"], "username": None, "credential": None},
            data[1],
        ])

    def test_rejects_unbounded_and_untrusted_ice(self):
        invalid = [
            "{}", "null", json.dumps([{}]),
            json.dumps([{"urls": ["https://not-an-ice-server"]}]),
            json.dumps([{"urls": ["turn:relay.example.org:3478"]}]),
            json.dumps([{"urls": ["stun:example.org"], "password": "wrong"}]),
            json.dumps([{"urls": ["stun:example.org"], "username": "only"}]),
            json.dumps([{"urls": ["stun:example.org"]}] * 9),
        ]
        for item in invalid:
            with self.subTest(value=item), self.assertRaises((ValueError, TypeError)):
                parse_ice_servers(item)


if __name__ == "__main__":
    unittest.main()
