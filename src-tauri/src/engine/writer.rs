//! 按 offset 写入文件；支持预分配与多段并发写

use bytes::Bytes;
use std::path::Path;
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
    // 打开已有文件时不得截断：暂停/恢复与失败重试都会重建 writer，
    // 截断会把已完成分段的数据清掉（pending_segments 不含已完成段）
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .await?;
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
