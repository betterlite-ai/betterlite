use crate::finops::vantage::destination::{
    MAX_BYTES_PER_UPLOAD, MAX_ROWS_PER_UPLOAD, VantageDestination,
};
use crate::finops::vantage::error::DeliveryError;
use crate::finops::vantage::records::is_blank;
use crate::finops::vantage::sender::CsvSender;

fn join_csv(header: &[u8], rows: &[&[u8]]) -> Vec<u8> {
    let mut csv = header.to_vec();
    csv.push(b'\n');
    for row in rows {
        csv.extend_from_slice(row);
        csv.push(b'\n');
    }
    csv
}

pub(super) async fn upload_batched<Sender>(
    destination: &VantageDestination<Sender>,
    records: &[&[u8]],
    filename: &str,
) -> Result<(), DeliveryError<Sender::Error>>
where
    Sender: CsvSender,
{
    let Some((header, rest)) = records.split_first() else {
        return Ok(());
    };
    let data: Vec<&[u8]> = rest
        .iter()
        .copied()
        .filter(|record| !is_blank(record))
        .collect();
    let mut first_error = None;
    for (batch_num, batch) in data.chunks(MAX_ROWS_PER_UPLOAD).enumerate() {
        let batch_csv = join_csv(header, batch);
        let result = if batch_csv.len() > MAX_BYTES_PER_UPLOAD {
            upload_size_limited(destination, header, batch, filename, batch_num).await
        } else {
            let batch_filename = format!("{filename}.part{batch_num}");
            destination.upload_csv(&batch_csv, &batch_filename).await
        };
        if let Err(error) = result {
            tracing::error!("Vantage destination: batch {batch_num} failed: {error}");
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

// A row that cannot fit next to the header is skipped; it cannot be split further.
async fn upload_size_limited<Sender>(
    destination: &VantageDestination<Sender>,
    header: &[u8],
    data: &[&[u8]],
    filename: &str,
    batch_offset: usize,
) -> Result<(), DeliveryError<Sender::Error>>
where
    Sender: CsvSender,
{
    let header_size = header.len().saturating_add(1);
    let mut chunk: Vec<&[u8]> = Vec::new();
    let mut chunk_size = header_size;
    let mut sub_batch = 0_usize;
    let mut first_error = None;
    for row in data {
        let row_size = row.len().saturating_add(1);
        if header_size.saturating_add(row_size) > MAX_BYTES_PER_UPLOAD {
            tracing::warn!("Vantage destination: skipping oversized row ({row_size} bytes)");
            continue;
        }
        if chunk_size.saturating_add(row_size) > MAX_BYTES_PER_UPLOAD && !chunk.is_empty() {
            let name = format!("{filename}.part{batch_offset}_{sub_batch}");
            upload_sub_batch(destination, header, &chunk, &name, &mut first_error).await;
            chunk.clear();
            chunk_size = header_size;
            sub_batch = sub_batch.saturating_add(1);
        }
        chunk.push(row);
        chunk_size = chunk_size.saturating_add(row_size);
    }
    if !chunk.is_empty() {
        let name = format!("{filename}.part{batch_offset}_{sub_batch}");
        upload_sub_batch(destination, header, &chunk, &name, &mut first_error).await;
    }
    first_error.map_or(Ok(()), Err)
}

async fn upload_sub_batch<Sender>(
    destination: &VantageDestination<Sender>,
    header: &[u8],
    chunk: &[&[u8]],
    name: &str,
    first_error: &mut Option<DeliveryError<Sender::Error>>,
) where
    Sender: CsvSender,
{
    if let Err(error) = destination.upload_csv(&join_csv(header, chunk), name).await {
        tracing::error!("Vantage destination: sub-batch {name} failed: {error}");
        first_error.get_or_insert(error);
    }
}
