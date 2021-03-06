//! The default implementation of a WebSocket Receiver.

use std::io::Read;
use std::io::Result as IoResult;

use hyper::buffer::BufReader;
use uuid::Uuid;

use dataframe::{DataFrame, Opcode};
use result::{WebSocketResult, WebSocketError};
use ws;
use ws::receiver::Receiver as ReceiverTrait;
use ws::receiver::{MessageIterator, DataFrameIterator};
use ws::util::header::{DataFrameHeader, ReaderState};
use message::OwnedMessage;
use stream::sync::{AsTcpStream, Stream};
pub use stream::sync::Shutdown;

#[derive(Debug, Default)]
/// A state for a reader to contain a buffer for incomplete reads to recover.
pub struct PacketState {
	/// The header of the dataframe.
	pub header: Option<DataFrameHeader>,
	/// The buffer of the packet to recover from.
	pub packet: Vec<u8>,
}

impl PacketState {
	/// Resets this state, setting [`header`] to `None` and clearing [`packet`].
	///
	/// [`header`]: #structfield.header
	/// [`packet`]: #structfield.packet
	pub fn reset(&mut self) {
		self.header = None;
		self.packet.clear();
	}
}

/// This reader bundles an existing stream with a parsing algorithm.
/// It is used by the client in its `.split()` function as the reading component.
pub struct Reader<R>
	where R: Read
{
	/// the stream to be read from
	pub stream: BufReader<R>,
	/// the parser to parse bytes into messages
	pub receiver: Receiver,
}

impl<R> Reader<R>
    where R: Read
{
	/// Reads a single data frame from the remote endpoint.
	pub fn recv_dataframe(&mut self) -> WebSocketResult<DataFrame> {
		let uuid = self.receiver.uuid;
		self.receiver.recv_dataframe(&mut self.stream, uuid)
	}

	/// Returns an iterator over incoming data frames.
	pub fn incoming_dataframes(&mut self) -> DataFrameIterator<Receiver, BufReader<R>> {
		self.receiver.incoming_dataframes(&mut self.stream)
	}

	/// Reads a single message from this receiver.
	pub fn recv_message(&mut self) -> WebSocketResult<OwnedMessage> {
		self.receiver.recv_message(&mut self.stream)
	}

	/// An iterator over incoming messsages.
	/// This iterator will block until new messages arrive and will never halt.
	pub fn incoming_messages<'a>(&'a mut self) -> MessageIterator<'a, Receiver, BufReader<R>> {
		self.receiver.incoming_messages(&mut self.stream)
	}
}

impl<S> Reader<S>
    where S: AsTcpStream + Stream + Read
{
	/// Closes the receiver side of the connection, will cause all pending and future IO to
	/// return immediately with an appropriate value.
	pub fn shutdown(&self) -> IoResult<()> {
		self.stream.get_ref().as_tcp().shutdown(Shutdown::Read)
	}

	/// Shuts down both Sender and Receiver, will cause all pending and future IO to
	/// return immediately with an appropriate value.
	pub fn shutdown_all(&self) -> IoResult<()> {
		self.stream.get_ref().as_tcp().shutdown(Shutdown::Both)
	}
}

/// A Receiver that wraps a Reader and provides a default implementation using
/// DataFrames and Messages.
pub struct Receiver {
	buffer: Vec<DataFrame>,
	mask: bool,
	packet_state: PacketState,
	reader_state: ReaderState,
	uuid: Uuid,
}

impl Receiver {
	/// Create a new Receiver using the specified Reader.
	pub fn new(mask: bool, uuid: Uuid) -> Receiver {
		Receiver {
			buffer: Vec::new(),
			mask: mask,
			packet_state: PacketState::default(),
			reader_state: ReaderState::new(),
			uuid: uuid,
		}
	}
}


impl ws::Receiver for Receiver {
	type F = DataFrame;

	type M = OwnedMessage;

	fn uuid(&self) -> Uuid {
		self.uuid
	}

	/// Reads a single data frame from the remote endpoint.
	fn recv_dataframe<R>(&mut self, reader: &mut R, uuid: Uuid) -> WebSocketResult<DataFrame>
		where R: Read
	{
		DataFrame::read_dataframe(
			reader,
			self.mask,
			uuid,
			&mut self.packet_state,
			&mut self.reader_state,
		)
	}

	/// Returns the data frames that constitute one message.
	fn recv_message_dataframes<R>(&mut self, reader: &mut R) -> WebSocketResult<Vec<DataFrame>>
		where R: Read
	{
		let uuid = self.uuid;
		let mut finished = if self.buffer.is_empty() {
			let first = self.recv_dataframe(reader, uuid)?;

			if first.opcode == Opcode::Continuation {
				return Err(WebSocketError::ProtocolError("Unexpected continuation data frame opcode",),);
			}

			let finished = first.finished;
			self.buffer.push(first);
			finished
		} else {
			false
		};

		while !finished {
			let next = self.recv_dataframe(reader, uuid)?;
			finished = next.finished;

			match next.opcode as u8 {
				// Continuation opcode
				0 => self.buffer.push(next),
				// Control frame
				8...15 => {
					return Ok(vec![next]);
				}
				// Others
				_ => return Err(WebSocketError::ProtocolError("Unexpected data frame opcode")),
			}
		}

		Ok(::std::mem::replace(&mut self.buffer, Vec::new()))
	}
}
