//! 按 offset 写入文件；支持预分配与多段并发写

use bytes::Bytes;
use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::mpsc;

pub type WriterMessage = (u64, Bytes);

/// 在后台任务中运行：接收 (offset, data) 并顺序写盘
pub async fn run_file_writer(
    path: impl AsRef<Path>,
    total_bytes: Option<u64>,
    mut rx: mpsc::Receiver<WriterMessage>,
) -> Result<(), std::io::Error> {
    let path = path.as_ref();
    let mut file = File::create(path).await?;
    if let Some(total) = total_bytes {
        file.set_len(total).await?;
    }
    while let Some((offset, data)) = rx.recv().await {
        file.seek(std::io::SeekFrom::Start(offset)).await?;
        file.write_all(&data).await?;
    }
    file.sync_all().await?;
    Ok(())
}
