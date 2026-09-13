//! M0 专用保活 OCR 子进程：有界内存帧、读写超时和作用域退出清理。

use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) struct OcrSidecar {
    child: Child,
    requests: Option<Sender<Vec<u8>>>,
    responses: Receiver<Result<Vec<u8>, String>>,
    io_thread: Option<JoinHandle<()>>,
}

impl OcrSidecar {
    pub(super) fn spawn() -> Result<Self, String> {
        let worker = std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join("python/dc_worker");
        let python = worker.join(".venv/Scripts/python.exe");
        if !python.is_file() {
            return Err("缺少 Worker 虚拟环境；请在 python/dc_worker 运行 uv sync --locked".into());
        }
        // 直接启动解释器：运行阶段不解析依赖、不联网，也不产生 uv 孙进程。
        let mut command = Command::new(python);
        command
            .args(["-u", "-m", "dc_worker", "ocr-serve"])
            .current_dir(worker)
            .env("PYTHONUTF8", "1")
            .env("PYTHONIOENCODING", "utf-8");
        Self::start(command, STARTUP_TIMEOUT)
    }

    fn start(mut command: Command, timeout: Duration) -> Result<Self, String> {
        let child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn()
            .map_err(|e| format!("无法启动 OCR 子进程: {e}"))?;
        let (request_tx, request_rx) = mpsc::channel::<Vec<u8>>();
        let (response_tx, response_rx) = mpsc::channel();
        // 先建立所有权，后续任一初始化错误均会 Drop 并回收进程。
        let mut sidecar = Self {
            child,
            requests: Some(request_tx),
            responses: response_rx,
            io_thread: None,
        };
        let mut stdout = sidecar.child.stdout.take().ok_or("缺少 OCR stdout")?;
        let mut stdin = sidecar.child.stdin.take().ok_or("缺少 OCR stdin")?;
        sidecar.io_thread = Some(
            std::thread::Builder::new()
                .name("m0-ocr-io".into())
                .spawn(move || {
                    let ready = read_frame(&mut stdout);
                    let failed = ready.is_err();
                    if response_tx.send(ready).is_err() || failed {
                        return;
                    }
                    for png in request_rx {
                        let response =
                            write_frame(&mut stdin, &png).and_then(|()| read_frame(&mut stdout));
                        let failed = response.is_err();
                        if response_tx.send(response).is_err() || failed {
                            return;
                        }
                    }
                })
                .map_err(|e| format!("无法创建 OCR IO 线程: {e}"))?,
        );
        if sidecar.receive(timeout)? != b"R" {
            return Err("OCR 子进程未就绪".into());
        }
        Ok(sidecar)
    }

    fn receive(&self, timeout: Duration) -> Result<Vec<u8>, String> {
        self.responses
            .recv_timeout(timeout)
            .map_err(|e| format!("OCR 读写超时或进程已退出: {e}"))?
    }

    pub(super) fn recognize(&mut self, png: Vec<u8>) -> Result<String, String> {
        self.recognize_with_timeout(png, REQUEST_TIMEOUT)
    }

    fn recognize_with_timeout(
        &mut self,
        png: Vec<u8>,
        timeout: Duration,
    ) -> Result<String, String> {
        if png.is_empty() || png.len() > MAX_IMAGE_BYTES {
            return Err("OCR 图像为空或超过 8 MiB".into());
        }
        let result = (|| {
            self.requests
                .as_ref()
                .ok_or("OCR 子进程已关闭")?
                .send(png)
                .map_err(|e| format!("OCR IO 线程已退出: {e}"))?;
            decode_result(self.receive(timeout)?)
        })();
        if result.is_err() {
            // 半帧、超时或识别失败后不复用管道，避免下一轮误读旧响应。
            self.stop();
        }
        result
    }

    fn stop(&mut self) {
        self.requests.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(thread) = self.io_thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for OcrSidecar {
    fn drop(&mut self) {
        self.stop();
    }
}

fn write_frame(writer: &mut impl Write, payload: &[u8]) -> Result<(), String> {
    writer
        .write_all(&(payload.len() as u32).to_le_bytes())
        .and_then(|()| writer.write_all(payload))
        .and_then(|()| writer.flush())
        .map_err(|e| format!("写 OCR 帧失败: {e}"))
}

fn read_frame(reader: &mut impl Read) -> Result<Vec<u8>, String> {
    let mut header = [0u8; 4];
    reader
        .read_exact(&mut header)
        .map_err(|e| format!("读取 OCR 帧头失败: {e}"))?;
    let size = u32::from_le_bytes(header) as usize;
    if size == 0 || size > MAX_RESPONSE_BYTES {
        return Err("OCR 响应长度无效".into());
    }
    let mut payload = vec![0; size];
    reader
        .read_exact(&mut payload)
        .map_err(|e| format!("读取 OCR 帧体失败: {e}"))?;
    Ok(payload)
}

fn decode_result(mut payload: Vec<u8>) -> Result<String, String> {
    if payload.first() != Some(&b'O') {
        return Err("OCR 识别失败或返回了未知响应".into());
    }
    payload.remove(0);
    String::from_utf8(payload).map_err(|_| "OCR 返回非 UTF-8 文本".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn rejects_eof_partial_and_oversized_frames() {
        for bytes in [
            vec![],
            vec![1],
            vec![2, 0, 0, 0, b'O'],
            vec![0; 4],
            ((MAX_RESPONSE_BYTES + 1) as u32).to_le_bytes().to_vec(),
        ] {
            assert!(read_frame(&mut Cursor::new(bytes)).is_err());
        }
    }

    #[test]
    fn distinguishes_empty_text_errors_and_multiline_text() {
        assert_eq!(decode_result(b"O".to_vec()).unwrap(), "");
        assert!(decode_result(b"EINFERENCE_FAILED".to_vec()).is_err());
        assert!(decode_result(b"R".to_vec()).is_err());
        assert!(decode_result(vec![b'O', 255]).is_err());
        let expected = "__ERROR__\n合成文本";
        let mut frame = Vec::new();
        write_frame(&mut frame, format!("O{expected}").as_bytes()).unwrap();
        assert_eq!(
            decode_result(read_frame(&mut Cursor::new(frame)).unwrap()).unwrap(),
            expected
        );
    }
}
