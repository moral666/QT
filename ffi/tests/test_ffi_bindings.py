"""
Testa os bindings FFI (gerados automaticamente pelo uniffi a partir do
Rust) a partir do PYTHON - a prova mais convincente de que o FFI funciona
de verdade fora do Rust, sem precisar de Android/iOS instalados. Kotlin e
Swift usam o mesmo mecanismo (uniffi), gerado da mesma forma.
"""
import sys
sys.path.insert(0, "ffi/bindings")
sys.path.insert(0, "target/debug")

import secure_messenger_ffi as ffi

print("=== Teste 1: geracao de chaves ===")
alice_identity = ffi.generate_dh_keypair()
bob_identity = ffi.generate_dh_keypair()
bob_signing = ffi.generate_signing_keypair()
bob_signed_pre_key = ffi.generate_dh_keypair()
bob_one_time_pre_key = ffi.generate_dh_keypair()
print(f"  Identidade de Alice gerada (chave publica, {len(alice_identity.public)} bytes)")
print(f"  Identidade de Bob gerada (chave publica, {len(bob_identity.public)} bytes)")

print("\n=== Teste 2: assinar a signed pre-key de Bob ===")
signature = ffi.sign_signed_pre_key(bob_signing.private, bob_signed_pre_key.public)
print(f"  Assinatura gerada ({len(signature)} bytes)")

print("\n=== Teste 3: X3DH (Alice inicia, Bob responde) ===")
init_result = ffi.x3dh_initiate(
    alice_identity.private,
    bob_identity.public,
    bob_signing.public,
    bob_signed_pre_key.public,
    signature,
    bob_one_time_pre_key.public,
)
print(f"  Alice calculou o segredo partilhado ({len(init_result.shared_secret)} bytes)")

bob_shared_secret = ffi.x3dh_respond(
    bob_identity.private,
    bob_signed_pre_key.private,
    bob_one_time_pre_key.private,
    alice_identity.public,
    init_result.ephemeral_public,
)

assert bytes(init_result.shared_secret) == bytes(bob_shared_secret), "OS SEGREDOS DEVIAM COINCIDIR"
print("  Segredo de Bob coincide com o de Alice. [OK]")

print("\n=== Teste 4: assinatura adulterada deve falhar ===")
try:
    assinatura_falsa = bytes([b ^ 0xFF for b in signature])
    ffi.x3dh_initiate(
        alice_identity.private, bob_identity.public, bob_signing.public,
        bob_signed_pre_key.public, assinatura_falsa, bob_one_time_pre_key.public,
    )
    print("  ERRO: deveria ter falhado mas nao falhou!")
    sys.exit(1)
except ffi.FfiError.InvalidSignature:
    print("  Rejeitado corretamente (InvalidSignature). [OK]")

print("\n=== Teste 5: Double Ratchet completo (Alice <-> Bob) ===")
alice_ratchet_state = ffi.ratchet_init_as_initiator(init_result.shared_secret, bob_signed_pre_key.public)
bob_ratchet_state = ffi.ratchet_init_as_responder(bob_shared_secret, bob_signed_pre_key.private)

mensagem = b"Ola Bob! Isto veio do Python, via bindings FFI gerados do Rust."
resultado_cifra = ffi.ratchet_encrypt(alice_ratchet_state, mensagem)
alice_ratchet_state = resultado_cifra.new_state
print(f"  Alice cifrou a mensagem ({len(resultado_cifra.ciphertext)} bytes de ciphertext)")

resultado_decifra = ffi.ratchet_decrypt(
    bob_ratchet_state, resultado_cifra.dh_public, resultado_cifra.n, resultado_cifra.ciphertext
)
bob_ratchet_state = resultado_decifra.new_state
texto_recebido = bytes(resultado_decifra.plaintext)

assert texto_recebido == mensagem, "A MENSAGEM DEVIA BATER CERTO"
print(f"  Bob decifrou: {texto_recebido.decode()!r} [OK]")

print("\n=== Teste 6: Sealed sender ===")
envelope = ffi.seal_sender(alice_identity.public, bob_identity.public)
print(f"  Envelope selado gerado ({len(envelope)} bytes)")

remetente_revelado = ffi.unseal_sender(bob_identity.private, envelope)
assert bytes(remetente_revelado) == bytes(alice_identity.public), "DEVERIA REVELAR ALICE"
print("  Bob abriu o envelope e confirmou que foi Alice. [OK]")

print("\n=== Teste 7: chave errada nao consegue abrir o envelope corretamente ===")
atacante_identity = ffi.generate_dh_keypair()
try:
    resultado_atacante = ffi.unseal_sender(atacante_identity.private, envelope)
    assert bytes(resultado_atacante) != bytes(alice_identity.public), "NUNCA deveria reconstruir a identidade certa"
    print("  Atacante nao conseguiu reconstruir a identidade correta (devolveu lixo). [OK]")
except ffi.FfiError.CryptoFailure:
    print("  Atacante nao conseguiu abrir o envelope (falhou a decifragem). [OK]")

print("\n>>> TODOS OS TESTES PASSARAM, A PARTIR DO PYTHON, VIA BINDINGS GERADOS DO RUST. <<<")
