use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

fn command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ulos"))
}

fn execute(command: &mut Command) -> Output {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("CLI command exceeded the 10-second fixture deadline");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn help_describes_the_actual_interface() {
    let output = execute(command().arg("--help"));
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("--output"));
    assert!(help.contains("HTTP(S)"));
}

#[test]
fn missing_output_is_an_argument_error() {
    let output = execute(command().arg("https://example.org/file"));
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn invalid_url_fails_without_creating_a_file() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("file.bin");
    let output = execute(command().args(["not-a-url", "--output"]).arg(&destination));
    assert_eq!(output.status.code(), Some(1));
    assert!(!destination.exists());
}

#[test]
fn downloads_exact_bytes_and_reports_success() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "Server accept exceeded the 5-second fixture deadline"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("Server accept failed: {error}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        {
            let mut reader = BufReader::new(&mut stream);
            let mut line = String::new();
            let mut length = 0;
            loop {
                line.clear();
                let count = reader.read_line(&mut line).unwrap();
                assert_ne!(count, 0, "Request ended before its headers");
                length += count;
                assert!(length < 16_384, "Request headers exceeded fixture limit");
                if line == "\r\n" {
                    break;
                }
            }
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello")
            .unwrap();
    });
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("file.bin");
    let output = execute(command().arg(url).arg("--output").arg(&destination));
    server.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read(destination).unwrap(), b"hello");
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("Saved 5 bytes")
    );
}

#[test]
fn network_errors_do_not_print_signed_queries() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let directory = tempfile::tempdir().unwrap();
    let output = execute(
        command()
            .arg(format!("http://{address}/file?token=secret-value"))
            .arg("--output")
            .arg(directory.path().join("file.bin")),
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("secret-value"));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}
