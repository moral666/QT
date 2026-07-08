"""
Prova de conceito: X3DH (simplificado) + Double Ratchet.

Objetivo: validar a LOGICA do protocolo com criptografia real (X25519, HKDF,
ChaCha20-Poly1305) antes de portar para o codigo de producao em Rust.
Isto NAO e a versao final - falta serializacao de rede, tratamento de
mensagens fora de ordem persistente, assinaturas Ed25519 na signed pre-key,
suporte PQXDH (Kyber), etc. Ver core/ (Rust) para a versao a evoluir.
"""

import os
from dataclasses import dataclass, field
from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey, X25519PublicKey
from cryptography.hazmat.primitives.kdf.hkdf import HKDF
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305


# ---------- Primitivas ----------

def dh(priv: X25519PrivateKey, pub: X25519PublicKey) -> bytes:
    return priv.exchange(pub)


def hkdf(key_material: bytes, info: bytes, length: int = 32, salt: bytes = b"") -> bytes:
    return HKDF(algorithm=hashes.SHA256(), length=length, salt=salt or None, info=info).derive(key_material)


def kdf_root_key(root_key: bytes, dh_output: bytes):
    """Deriva (novo root key, nova chain key) a partir do root key atual + novo DH."""
    output = hkdf(dh_output, info=b"DoubleRatchet_RootKDF", length=64, salt=root_key)
    return output[:32], output[32:]


def kdf_chain_key(chain_key: bytes):
    """Deriva (message key, proxima chain key) a partir da chain key atual."""
    message_key = hkdf(chain_key, info=b"DoubleRatchet_MsgKey", length=32)
    next_chain_key = hkdf(chain_key, info=b"DoubleRatchet_ChainKey", length=32)
    return message_key, next_chain_key


def aead_encrypt(key: bytes, plaintext: bytes, aad: bytes = b"") -> bytes:
    nonce = os.urandom(12)
    ct = ChaCha20Poly1305(key).encrypt(nonce, plaintext, aad)
    return nonce + ct


def aead_decrypt(key: bytes, blob: bytes, aad: bytes = b"") -> bytes:
    nonce, ct = blob[:12], blob[12:]
    return ChaCha20Poly1305(key).decrypt(nonce, ct, aad)


# ---------- X3DH simplificado (sem one-time pre-key / sem assinatura, para clareza) ----------

@dataclass
class IdentityKeyPair:
    private: X25519PrivateKey
    public: X25519PublicKey = field(init=False)

    def __post_init__(self):
        self.public = self.private.public_key()


def generate_identity() -> IdentityKeyPair:
    return IdentityKeyPair(private=X25519PrivateKey.generate())


def x3dh_initiator(my_identity: IdentityKeyPair, their_identity_pub: X25519PublicKey,
                    their_signed_prekey_pub: X25519PublicKey):
    """Alice inicia. Retorna (shared_secret, ephemeral_public_para_enviar_ao_Bob)."""
    ephemeral = X25519PrivateKey.generate()

    dh1 = dh(my_identity.private, their_signed_prekey_pub)   # IK_A x SPK_B
    dh2 = dh(ephemeral, their_identity_pub)                  # EK_A x IK_B
    dh3 = dh(ephemeral, their_signed_prekey_pub)             # EK_A x SPK_B

    combined = dh1 + dh2 + dh3
    shared_secret = hkdf(combined, info=b"X3DH_v1", length=32)
    return shared_secret, ephemeral.public_key()


def x3dh_responder(my_identity: IdentityKeyPair, my_signed_prekey: X25519PrivateKey,
                    their_identity_pub: X25519PublicKey, their_ephemeral_pub: X25519PublicKey):
    """Bob recebe a primeira mensagem e deriva o mesmo segredo."""
    dh1 = dh(my_signed_prekey, their_identity_pub)
    dh2 = dh(my_identity.private, their_ephemeral_pub)
    dh3 = dh(my_signed_prekey, their_ephemeral_pub)

    combined = dh1 + dh2 + dh3
    shared_secret = hkdf(combined, info=b"X3DH_v1", length=32)
    return shared_secret


# ---------- Double Ratchet ----------

@dataclass
class RatchetState:
    root_key: bytes
    dh_send_priv: X25519PrivateKey
    dh_recv_pub: X25519PublicKey | None = None
    sending_chain_key: bytes | None = None
    receiving_chain_key: bytes | None = None
    send_n: int = 0
    recv_n: int = 0
    skipped: dict = field(default_factory=dict)  # (pub_bytes, n) -> message_key


def ratchet_init_alice(shared_secret: bytes, bob_dh_pub: X25519PublicKey) -> RatchetState:
    """Alice inicializa apos X3DH, ja conhecendo a signed pre-key publica de Bob
    como a primeira chave DH 'recebida' (simplificacao valida para a 1a mensagem)."""
    state = RatchetState(root_key=shared_secret, dh_send_priv=X25519PrivateKey.generate())
    dh_out = dh(state.dh_send_priv, bob_dh_pub)
    new_root, sending_chain = kdf_root_key(state.root_key, dh_out)
    state.root_key = new_root
    state.sending_chain_key = sending_chain
    state.dh_recv_pub = bob_dh_pub
    return state


def ratchet_init_bob(shared_secret: bytes, bob_signed_prekey_priv: X25519PrivateKey) -> RatchetState:
    state = RatchetState(root_key=shared_secret, dh_send_priv=bob_signed_prekey_priv)
    return state


def dh_ratchet_step(state: RatchetState, their_new_dh_pub: X25519PublicKey):
    dh_out = dh(state.dh_send_priv, their_new_dh_pub)
    new_root, recv_chain = kdf_root_key(state.root_key, dh_out)
    state.root_key = new_root
    state.receiving_chain_key = recv_chain
    state.dh_recv_pub = their_new_dh_pub
    state.recv_n = 0

    state.dh_send_priv = X25519PrivateKey.generate()
    dh_out2 = dh(state.dh_send_priv, their_new_dh_pub)
    new_root2, send_chain = kdf_root_key(state.root_key, dh_out2)
    state.root_key = new_root2
    state.sending_chain_key = send_chain
    state.send_n = 0


def ratchet_encrypt(state: RatchetState, plaintext: bytes) -> dict:
    message_key, next_chain = kdf_chain_key(state.sending_chain_key)
    state.sending_chain_key = next_chain
    header_pub = state.dh_send_priv.public_key()
    n = state.send_n
    state.send_n += 1
    aad = header_pub.public_bytes_raw() + n.to_bytes(4, "big")
    ciphertext = aead_encrypt(message_key, plaintext, aad)
    return {"dh_pub": header_pub, "n": n, "ciphertext": ciphertext}


def ratchet_decrypt(state: RatchetState, msg: dict) -> bytes:
    incoming_pub = msg["dh_pub"]
    if state.dh_recv_pub is None or incoming_pub.public_bytes_raw() != state.dh_recv_pub.public_bytes_raw():
        dh_ratchet_step(state, incoming_pub)

    key = (incoming_pub.public_bytes_raw(), msg["n"])
    if key in state.skipped:
        message_key = state.skipped.pop(key)
    else:
        while state.recv_n < msg["n"]:
            mk, next_chain = kdf_chain_key(state.receiving_chain_key)
            state.skipped[(incoming_pub.public_bytes_raw(), state.recv_n)] = mk
            state.receiving_chain_key = next_chain
            state.recv_n += 1
        message_key, next_chain = kdf_chain_key(state.receiving_chain_key)
        state.receiving_chain_key = next_chain
        state.recv_n += 1

    aad = incoming_pub.public_bytes_raw() + msg["n"].to_bytes(4, "big")
    return aead_decrypt(message_key, msg["ciphertext"], aad)


# ---------- Teste end-to-end ----------

def run_demo():
    print("=== Setup de identidades ===")
    alice_id = generate_identity()
    bob_id = generate_identity()
    bob_signed_prekey = X25519PrivateKey.generate()
    print("Identidades e pre-key de Bob geradas.\n")

    print("=== X3DH: Alice estabelece segredo com o bundle publico de Bob ===")
    shared_alice, alice_ephemeral_pub = x3dh_initiator(
        alice_id, bob_id.public, bob_signed_prekey.public_key()
    )
    shared_bob = x3dh_responder(
        bob_id, bob_signed_prekey, alice_id.public, alice_ephemeral_pub
    )
    assert shared_alice == shared_bob, "X3DH FALHOU: segredos nao coincidem"
    print("Segredo compartilhado (X3DH) estabelecido e verificado em ambos os lados.\n")

    print("=== Inicializando Double Ratchet ===")
    alice_state = ratchet_init_alice(shared_alice, bob_signed_prekey.public_key())
    bob_state = ratchet_init_bob(shared_bob, bob_signed_prekey)
    print("Estados de ratchet inicializados.\n")

    print("=== Alice envia 3 mensagens para Bob ===")
    msgs_texto = ["Ola Bob, tudo bem?", "Isto e uma prova de conceito.", "Terceira mensagem."]
    enviados = [ratchet_encrypt(alice_state, m.encode()) for m in msgs_texto]

    for i, m in enumerate(enviados):
        decrypted = ratchet_decrypt(bob_state, m)
        print(f"  Msg {i}: Bob decifrou -> {decrypted.decode()!r}  [OK]" if decrypted.decode() == msgs_texto[i] else "FALHOU")

    print("\n=== Bob responde (isto forca um DH ratchet step - nova geracao de chaves) ===")
    resposta = ratchet_encrypt(bob_state, b"Recebi tudo, forward secrecy funcionando!")
    decrypted_resposta = ratchet_decrypt(alice_state, resposta)
    print(f"  Alice decifrou -> {decrypted_resposta.decode()!r}")
    assert decrypted_resposta == b"Recebi tudo, forward secrecy funcionando!"

    print("\n=== Teste de mensagem fora de ordem (simula pacote perdido/atrasado) ===")
    m1 = ratchet_encrypt(alice_state, b"mensagem A")
    m2 = ratchet_encrypt(alice_state, b"mensagem B")
    # Bob recebe m2 primeiro, depois m1 (fora de ordem)
    dec_m2 = ratchet_decrypt(bob_state, m2)
    dec_m1 = ratchet_decrypt(bob_state, m1)
    assert dec_m2 == b"mensagem B" and dec_m1 == b"mensagem A"
    print("  Mensagens fora de ordem decifradas corretamente via skipped-keys. [OK]")

    print("\n>>> TODOS OS TESTES PASSARAM. Logica validada. <<<")


if __name__ == "__main__":
    run_demo()
