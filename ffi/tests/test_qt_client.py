"""Testa a API de alto nivel (QtClient) pelos bindings gerados - o mesmo
mecanismo UniFFI que produz o Kotlin da app Android."""
import os, sys, tempfile
sys.path.insert(0, "ffi/bindings")
sys.path.insert(0, "target/debug")
import qt_ffi as q

d = tempfile.mkdtemp()
ka, kb = bytes([1]) * 32, bytes([2]) * 32
pa, pb = os.path.join(d, "a.db"), os.path.join(d, "b.db")
alice, bob = q.QtClient.open(pa, ka), q.QtClient.open(pb, kb)
ida, idb = alice.create_identity(), bob.create_identity()
b = bob.generate_prekey_bundle(3)
peer = q.PeerBundle(identity_public=b.identity_public, signing_public=b.signing_public,
    signed_pre_key_public=b.signed_pre_key_public, signed_pre_key_signature=b.signed_pre_key_signature,
    pq_pre_key_id=b.pq_pre_key_id, pq_pre_key_public=b.pq_pre_key_public,
    one_time_pre_key=b.one_time_pre_keys[0])
bob_id = alice.start_session(idb.public_id, peer)
alice_id = ida.identity_public.hex()

env = alice.encrypt(bob_id, b"ola via python")
assert bob.decrypt(alice_id, env) == b"ola via python"
try:
    bob.decrypt(alice_id, env); assert False, "replay aceite"
except q.QtError.Replay:
    pass
# reinicio de Bob e continuacao
bob = q.QtClient.open(pb, kb)
assert alice.decrypt(bob_id, bob.encrypt(alice_id, b"resposta")) == b"resposta"
# selado
sealed = alice.encrypt_sealed(bob_id, b"selada")
r = bob.decrypt_sealed(sealed)
assert r.plaintext == b"selada" and r.contact_id == alice_id
# chave errada
try:
    q.QtClient.open(pb, bytes(32)); assert False
except q.QtError:
    pass
print("  QtClient (PQXDH + ratchet + sealed + persistencia + replay) [OK]")
