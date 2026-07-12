import sys
sys.path.insert(0, "ffi/bindings")

import qt_ffi as ffi

print("=== Teste do NoiseSession (handshake + transporte) ===")

initiator_keys = ffi.generate_noise_static_keypair()
responder_keys = ffi.generate_noise_static_keypair()

initiator = ffi.NoiseSession.new_initiator(initiator_keys.private)
responder = ffi.NoiseSession.new_responder(responder_keys.private)

# Padrao Noise_XX: 3 mensagens. Iniciador escreve, le, escreve.
# Respondente le, escreve, le.
msg1 = initiator.write_step()
responder.read_step(msg1)

msg2 = responder.write_step()
initiator.read_step(msg2)

msg3 = initiator.write_step()
responder.read_step(msg3)

print(f"Handshake terminado - initiator.is_finished()={initiator.is_finished()}, responder.is_finished()={responder.is_finished()}")
assert initiator.is_finished() and responder.is_finished()

print("\n=== Trocar mensagens de aplicacao cifradas ===")
mensagem = b"Ola atraves do Noise, testado via Python antes de ir para Android!"
ciphertext = initiator.encrypt(mensagem)
print(f"Cifrado: {len(ciphertext)} bytes")

plaintext = responder.decrypt(bytes(ciphertext))
assert bytes(plaintext) == mensagem
print(f"Decifrado: {bytes(plaintext).decode()!r}")

# Resposta na direcao inversa
resposta = responder.encrypt(b"Recebido!")
resposta_decifrada = initiator.decrypt(bytes(resposta))
assert bytes(resposta_decifrada) == b"Recebido!"
print(f"Resposta decifrada pelo iniciador: {bytes(resposta_decifrada).decode()!r}")

print("\n>>> NOISE SESSION FUNCIONA DE PONTA A PONTA VIA FFI. <<<")
