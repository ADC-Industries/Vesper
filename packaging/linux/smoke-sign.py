#!/usr/bin/env python3
# A throwaway minisign key for the smoke test only (never the release key, which is kept offline):
#   smoke-sign.py OUT_DIR PACKAGE PRODUCT VERSION PLATFORM
# writes OUT_DIR/smoke.pub (the public key) and OUT_DIR/<package name>.minisig (a prehashed
# Ed25519 signature with the trusted comment "vesper-package product=… version=… platform=…").
# Needs python3-cryptography.
import base64
import hashlib
import os
import sys

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat


def main() -> None:
    out, package, product, version, platform = sys.argv[1:6]
    key = Ed25519PrivateKey.generate()
    key_id = os.urandom(8)
    public = key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
    with open(os.path.join(out, "smoke.pub"), "w") as f:
        f.write("untrusted comment: smoke-test key\n")
        f.write(base64.b64encode(b"Ed" + key_id + public).decode() + "\n")
    h = hashlib.blake2b()
    with open(package, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    signature = key.sign(h.digest())
    comment = f"vesper-package product={product} version={version} platform={platform}"
    global_sig = key.sign(signature + comment.encode())
    with open(os.path.join(out, os.path.basename(package) + ".minisig"), "w") as f:
        f.write("untrusted comment: signature from the smoke-test key\n")
        f.write(base64.b64encode(b"ED" + key_id + signature).decode() + "\n")
        f.write(f"trusted comment: {comment}\n")
        f.write(base64.b64encode(global_sig).decode() + "\n")


if __name__ == "__main__":
    main()
