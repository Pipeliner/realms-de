import unittest

from portal_vm_helper import frame_evidence, request_path, single_stream_node


class PortalVmHelperContract(unittest.TestCase):
    def test_request_path_matches_portal_sender_convention(self):
        self.assertEqual(
            request_path(":1.42", "realm_file"),
            "/org/freedesktop/portal/desktop/request/1_42/realm_file",
        )

    def test_request_path_rejects_non_unique_bus_name(self):
        with self.assertRaises(ValueError):
            request_path("org.example.Client", "realm_file")

    def test_single_stream_requires_exactly_one_positive_node(self):
        self.assertEqual(single_stream_node([(41, {})]), 41)
        for streams in [[], [(0, {})], [(4, {}), (5, {})]]:
            with self.subTest(streams=streams), self.assertRaises(ValueError):
                single_stream_node(streams)

    def test_frame_evidence_refuses_metadata_only_success(self):
        self.assertEqual(
            frame_evidence(41, 4096, 1280, 800),
            {
                "node_id": 41,
                "buffer_bytes": 4096,
                "width": 1280,
                "height": 800,
            },
        )
        for size, width, height in [(0, 1280, 800), (1, 0, 800), (1, 1280, 0)]:
            with self.subTest(size=size, width=width, height=height), self.assertRaises(
                ValueError
            ):
                frame_evidence(41, size, width, height)


if __name__ == "__main__":
    unittest.main()
