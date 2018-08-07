use std::io;
use std::error::Error;
use std::fmt;
use std::result::Result;
use std::vec::Vec;
use std::str::from_utf8;
use tokio::prelude::{AsyncRead};
use tokio::io::{read_until, read_exact};
use futures::{future, Future};
use super::resp::{Resp, BulkStr, BinSafeStr, Array};

#[derive(Debug)]
pub enum DecodeError {
    InvalidProtocol,
    Io(io::Error),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl Error for DecodeError {
    fn description(&self) -> &str {
        "I'm the superhero of errors"
    }

    fn cause(&self) -> Option<&Error> {
        match self {
            DecodeError::Io(err) => Some(err),
            _ => None,
        }
    }
}

type ParseResult = Result<(), DecodeError>;

const LF: u8 = '\n' as u8;

fn bytes_to_int(bytes: &[u8]) -> Result<i64, DecodeError> {
    const ZERO : u8 = '0' as u8;
    let mut it = bytes.iter().peekable();
    let f = match it.peek().map(|first| {**first as char}) {
        Some('-') => {
            it.next();
            -1
        },
        _ => 1,
    };
    it.fold(Ok(0), |sum, i| {
        match *i as char {
            '0' ... '9' => sum.map(|s| s * 10 + (i - ZERO) as i64),
            _ => Err(DecodeError::InvalidProtocol),
        }
    }).map(|i| i * f)
}

fn decode_len<R>(reader: R) -> impl Future<Item = (R, i64), Error = DecodeError>
    where R: AsyncRead + io::BufRead
{
    decode_line(reader).and_then(|(reader, s)| {
        let len_res = bytes_to_int(&s[..]).map(|l| (reader, l));
        future::result(len_res)
    })
}

fn decode_bulk_str<R>(reader: R) -> impl Future<Item = (R, BulkStr), Error = DecodeError>
    where R: AsyncRead + io::BufRead
{
    decode_len(reader)
        .and_then(|(reader, len)| {
            let read_len = if len < 0 {0} else {len as usize + 2};  // add CRLF
            read_exact(reader, vec![0; read_len])
                .map_err(DecodeError::Io)
                .and_then(move |(reader, line)| {
                    if len < 0 {
                        return future::ok((reader, BulkStr::Nil))
                    }
                    let mut s = line;
                    s.truncate(len as usize);
                    future::ok((reader, BulkStr::Str(s)))
                })
        })
}

fn decode_line<R>(reader: R) -> impl Future<Item = (R, BinSafeStr), Error = DecodeError>
    where R: AsyncRead + io::BufRead
{
    read_until(reader, LF, vec![])
        .map_err(DecodeError::Io)
        .and_then(|(reader, line)| {
            let len = line.len();
            if len <= 2 {
                return future::err(DecodeError::InvalidProtocol)
            }
            let mut s = line;
            s.truncate(len - 2);
            future::ok((reader, s))
        })
}

fn decode_resp<R>(reader: R) -> impl Future<Item = (R, Resp), Error = DecodeError>
    where R: AsyncRead + io::BufRead + 'static
{
    read_exact(reader, vec![0; 1])
        .map_err(DecodeError::Io)
        .and_then(|(reader, prefix)| {
            let bf: Box<Future<Item = (R, Resp), Error = DecodeError>> = match prefix[0] as char {
                '$' => {
                    Box::new(decode_bulk_str(reader)
                        .and_then(|(reader, s)| future::ok((reader, Resp::Bulk(s)))))
                }
                '+' => {
                    Box::new(decode_line(reader)
                        .and_then(|(reader, s)| future::ok((reader, Resp::Simple(s)))))
                }
                ':' => {
                    Box::new(decode_line(reader)
                        .and_then(|(reader, s)| future::ok((reader, Resp::Integer(s)))))
                }
                '-' => {
                    Box::new(decode_line(reader)
                        .and_then(|(reader, s)| future::ok((reader, Resp::Error(s)))))
                }
                '*' => {
                    Box::new(decode_array(reader)
                        .and_then(|(reader, a)| future::ok((reader, Resp::Arr(a)))))
                }
                _ => Box::new(future::err(DecodeError::InvalidProtocol)),
            };
            bf
        })
}

fn decode_array<R>(reader: R) -> impl Future<Item = (R, Array), Error = DecodeError>
    where R: AsyncRead + io::BufRead + 'static
{
    unimplemented!();
    future::ok((reader, Array::Nil))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bytes_to_int() {
        assert_eq!(233, bytes_to_int("233".as_bytes()).unwrap());
        assert_eq!(-233, bytes_to_int("-233".as_bytes()).unwrap());
        assert!(bytes_to_int("233a".as_bytes()).is_err())
    }

    #[test]
    fn test_decode_len() {
        let c = io::Cursor::new("233\r\n".as_bytes());
        let r = decode_len(c).wait();
        assert!(r.is_ok());
        let (_, l) = r.unwrap();
        assert_eq!(l, 233);

        let c = io::Cursor::new("-233\r\n".as_bytes());
        let r = decode_len(c).wait();
        assert!(r.is_ok());
        let (_, l) = r.unwrap();
        assert_eq!(l, -233);

        let c = io::Cursor::new("2a3\r\n".as_bytes());
        let r = decode_len(c).wait();
        assert!(r.is_err());
    }

    #[test]
    fn test_decode_bulk_str() {
        let c = io::Cursor::new("2\r\nab\r\n".as_bytes());
        let r = decode_bulk_str(c).wait();
        assert!(r.is_ok());
        let (_, s) = r.unwrap();
        assert_eq!(BulkStr::Str(String::from("ab").into_bytes()), s);

        let c = io::Cursor::new("-1\r\n".as_bytes());
        let r = decode_bulk_str(c).wait();
        assert!(r.is_ok());
        let (_, s) = r.unwrap();
        assert_eq!(BulkStr::Nil, s);

        let c = io::Cursor::new("2a3\r\nab\r\n".as_bytes());
        let r = decode_bulk_str(c).wait();
        assert!(r.is_err());
    }

    #[test]
    fn test_decode_line() {
        let c = io::Cursor::new("233\r\n".as_bytes());
        let r = decode_line(c).wait();
        assert!(r.is_ok());
        let (_, l) = r.unwrap();
        assert_eq!(from_utf8(&l[..]), Ok("233"));

        let c = io::Cursor::new("-233\r\n".as_bytes());
        let r = decode_line(c).wait();
        assert!(r.is_ok());
        let (_, l) = r.unwrap();
        assert_eq!(from_utf8(&l[..]), Ok("-233"));
    }
}