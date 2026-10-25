//! One bounded line from a stream: in the admin, an answer on the agent's link and a message on the local socket; in
//! the controller, a request.

use tokio::io::{AsyncBufRead, AsyncBufReadExt};

pub enum Line {
    /// A whole line, without its line feed.
    Complete(Vec<u8>),
    /// A line longer than the limit, read only up to it: the rest of it is still in the stream.
    TooLong,
    /// The input ended, maybe in the middle of a line: a line cut by the end of input is not a message.
    Closed,
}

/// Reads one line of at most `limit` bytes (without its line feed), never buffering more than that: its buffer grows to
/// the next power of two of what it holds, up to `limit`, so a line just past a power of two never takes twice its
/// length.
pub async fn read<R: AsyncBufRead + Unpin>(reader: &mut R, limit: usize) -> std::io::Result<Line> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(Line::Closed);
        }
        let end = available.iter().position(|&byte| byte == b'\n');
        let piece = &available[..end.unwrap_or(available.len())];
        let length = line.len() + piece.len();
        if length > limit {
            return Ok(Line::TooLong);
        }
        if length > line.capacity() {
            line.reserve_exact(length.next_power_of_two().min(limit) - line.len());
        }
        line.extend_from_slice(piece);
        let read = piece.len() + usize::from(end.is_some());
        reader.consume(read);
        if end.is_some() {
            return Ok(Line::Complete(line));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn lines(input: &[u8], limit: usize, chunk: usize) -> Vec<Result<Vec<u8>, &'static str>> {
        let mut reader = tokio::io::BufReader::with_capacity(chunk, input);
        let mut lines = Vec::new();
        loop {
            match read(&mut reader, limit).await.unwrap() {
                Line::Complete(line) => lines.push(Ok(line)),
                Line::TooLong => return [lines, vec![Err("too long")]].concat(),
                Line::Closed => return lines,
            }
        }
    }

    #[tokio::test]
    async fn lines_up_to_the_limit_are_read_whole_in_pieces_and_a_longer_one_or_a_cut_one_is_not() {
        for chunk in [1, 3, 64] {
            let read = lines(b"abc\n\nabcd\nab", 4, chunk).await;
            assert_eq!(read, [Ok(b"abc".to_vec()), Ok(vec![]), Ok(b"abcd".to_vec())], "chunks of {chunk}");
            let read = lines(b"abcd\nabcde\nabc\n", 4, chunk).await;
            assert_eq!(read, [Ok(b"abcd".to_vec()), Err("too long")], "chunks of {chunk}");
            // Past the limit is too long as soon as it is read, even when the input then ends mid-line.
            assert_eq!(lines(b"abcde", 4, chunk).await, [Err("too long")], "chunks of {chunk}");
        }
    }

    #[tokio::test]
    async fn a_line_s_buffer_grows_to_a_power_of_two_and_never_past_the_limit() {
        let capacity = |length: usize, limit: usize| async move {
            let input = [vec![b'a'; length], vec![b'\n']].concat();
            let mut reader = tokio::io::BufReader::with_capacity(1000, &input[..]);
            let Line::Complete(line) = read(&mut reader, limit).await.unwrap() else { panic!("a line") };
            assert_eq!(line.len(), length);
            line.capacity()
        };
        assert_eq!(capacity(5000, 1 << 20).await, 8192);
        assert_eq!(capacity(5000, 6000).await, 6000, "a limit that is no power of two");
        assert_eq!(capacity(4096, 1 << 20).await, 4096);
    }
}
