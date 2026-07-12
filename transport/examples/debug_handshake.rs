use qt_transport::{generate_static_keypair, NoiseHandshake};

fn main() {
    let init_keys = generate_static_keypair().unwrap();
    let resp_keys = generate_static_keypair().unwrap();

    let mut initiator = NoiseHandshake::new_initiator(&init_keys.private).unwrap();
    let mut responder = NoiseHandshake::new_responder(&resp_keys.private).unwrap();

    println!("inicio: initiator.finished={} responder.finished={}", initiator.is_finished(), responder.is_finished());

    let msg1 = initiator.write_step().unwrap();
    println!("apos initiator write msg1: initiator.finished={}", initiator.is_finished());

    responder.read_step(&msg1).unwrap();
    println!("apos responder read msg1: responder.finished={}", responder.is_finished());

    let msg2 = responder.write_step().unwrap();
    println!("apos responder write msg2: responder.finished={}", responder.is_finished());

    initiator.read_step(&msg2).unwrap();
    println!("apos initiator read msg2: initiator.finished={}", initiator.is_finished());

    if !initiator.is_finished() {
        let msg3 = initiator.write_step().unwrap();
        println!("apos initiator write msg3: initiator.finished={}", initiator.is_finished());

        responder.read_step(&msg3).unwrap();
        println!("apos responder read msg3: responder.finished={}", responder.is_finished());
    }

    println!("FIM: initiator.finished={} responder.finished={}", initiator.is_finished(), responder.is_finished());
}
