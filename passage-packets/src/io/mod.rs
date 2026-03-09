pub mod codec;
pub mod reader;
pub mod writer;

#[cfg(test)]
pub(crate) mod tests {
    use super::reader::ReadPacket;
    use super::writer::WritePacket;
    use crate::VarInt;
    use fake::{Dummy, Fake, Faker};
    use std::fmt::Debug;
    use std::io::Cursor;

    pub fn assert_packet<T>(packet_id: VarInt)
    where
        T: PartialEq + Eq + Dummy<Faker> + ReadPacket + WritePacket + Send + Sync + Debug + Clone,
    {
        // generate data
        let expected: T = Faker.fake();

        // write packets
        let mut writer: Cursor<Vec<u8>> = Cursor::new(Vec::new());
        expected
            .write_packet(&mut writer)
            .expect("failed to write packets");

        // read packets
        let mut reader: Cursor<Vec<u8>> = Cursor::new(writer.into_inner());
        let actual = T::read_packet(&mut reader).expect("failed to read packets");

        assert_eq!(T::ID, packet_id, "mismatching packet id");
        assert_eq!(expected, actual);
        assert_eq!(
            reader.position() as usize,
            reader.get_ref().len(),
            "there are remaining bytes in the buffer"
        );
    }
}
