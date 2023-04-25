use super::{data_type::DataBlock, device_info::DeviceInfo, BlockDevice};
use crate::block_device::data_type::{BLOCK_SIZE, UNMAP_BLOCK};
use std::io::{Seek, Write};
//use std::os::fd::AsRawFd 를 못찾음
use std::os::unix::io::AsRawFd;

const URING_SIZE: u32 = 8;

#[cfg(target_os = "linux")]
pub struct IoUringFakeDevice {
    device_info: DeviceInfo,
    ring: io_uring::IoUring,
}

#[cfg(target_os = "linux")]
impl IoUringFakeDevice {
    pub fn new(name: String, size: u64) -> Result<Self, String> {
        let device_info = DeviceInfo::new(name, size)?;
        let ring = io_uring::IoUring::new(URING_SIZE).map_err(|e| e.to_string())?;

        let file_name: &str = device_info.name();
        let mut fd = std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .open(file_name)
                    .map_err(|e| e.to_string())?;
        // device info 의 block 개수만큼 초기화
        for lba in 0..device_info.num_blocks() {
            fd.seek(std::io::SeekFrom::Start(lba * BLOCK_SIZE as u64))
                .map_err(|e| e.to_string())?;
            fd.write_all(&UNMAP_BLOCK.0).map_err(|e| e.to_string())?;

        }
        // create files to write/read
        Ok(Self { device_info, ring })
    }

    fn is_valid_range(&self, lba: u64, num_blocks: u64) -> bool {
        if lba + num_blocks > self.device_info.num_blocks() {
            return false
        }
        else {
            return true
        }
    }
}

#[cfg(target_os = "linux")]
impl BlockDevice for IoUringFakeDevice {
    fn info(&self) -> &DeviceInfo {
        &self.device_info
    }

    fn write(&mut self, lba: u64, num_blocks: u64, buffer: Vec<DataBlock>) -> Result<(), String> {
        // check lba range
        if self.is_valid_range(lba, num_blocks) == false {
            return Err("The number of blocks does not match with buffer".to_string())
        }
        if buffer.len() as u64 != num_blocks {
            return Err("The number of blocks does not match with buffer".to_string())
        }
        // make buffer to write
        let file_name = self.device_info.name();
        //이거랑 무슨 차이? let fd = std::fs::File::open(file_name);
        let fd = std::fs::OpenOptions::new()
            .write(true)
            .open(file_name)
            .map_err(|e| e.to_string())?;

        let bytes_ptr: *const u8 = buffer.as_ptr() as *const u8;
        let write_e = io_uring::opcode::Write::new(
            io_uring::types::Fd(fd.as_raw_fd()),
            bytes_ptr , (buffer.len() * std::mem::size_of::<DataBlock>()) as u32)
            .offset((lba * BLOCK_SIZE as u64) as i64) // offset 문서에서는 offset: u64인데 왜 여긴 i64...?
            .build();

        unsafe {
            self.ring
                .submission()
                .push(&write_e)
                .map_err(|e| e.to_string())?;
        }

        self.ring
            .submit_and_wait(1)
            .expect("failed to submit");

        if let Some(cqe) = self.ring.completion().next(){
            if cqe.result() == 0 {
                Ok(())
            }
            else{
                Err("Write Completion Failed".to_string())
            }
        }
        else{
            Err("Write Completion Failed".to_string())
        }
    }

    fn read(&mut self, lba: u64, num_blocks: u64) -> Result<Vec<DataBlock>, String> {
        if self.is_valid_range(lba, num_blocks) == false {
            return Err("The number of blocks does not match with buffer".to_string())
        }
        let file_name = self.device_info.name();
        let fd = std::fs::OpenOptions::new()
                    .read(true)
                    .open(file_name)
                    .map_err(|e| e.to_string())?; // ? 의 역할은?

        
        let temp = Vec::new();
        Ok(temp)
    }
    fn load(&mut self) -> Result<(), String> {
        // Do nothing as data will be read from the file
        Ok(())
    }
    fn flush(&mut self) -> Result<(), String> {
        // Do nothing as data is saved in file
        Ok(())
    }
}

#[cfg(target_os = "linux")]
#[cfg(test)]
mod tests {
    use io_uring::{opcode, types, IoUring};
    use std::fs;
    use std::os::unix::io::AsRawFd;
    use std::panic;
    use std::path::Path;

    fn panic_hook(info: &panic::PanicInfo<'_>) {
        println!("Panic occurred: {:?}", info);
        let path = Path::new("text.txt");
        if path.try_exists().unwrap() {
            fs::remove_file(path).unwrap();
        }
    }
    #[test]
    pub fn simple_uring_test_on_linux() {
        panic::set_hook(Box::new(panic_hook));
        let mut ring = IoUring::new(8).expect("Failed to create IoUring");

        let file_name = "text.txt";
        let fd = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(file_name.clone())
            .expect("Failed to open file");
        // Write data to the file
        {
            let mut buf: [u8; 1024] = [0xA; 1024];
            let write_e: io_uring::squeue::Entry =
                opcode::Write::new(types::Fd(fd.as_raw_fd()), buf.as_mut_ptr(), buf.len() as _)
                    .build();

            unsafe {
                ring.submission()
                    .push(&write_e)
                    .expect("submission queue is full");
            }

            ring.submit_and_wait(1)
                .expect("Failed to submit write request to ring");
            let cqe = ring.completion().next().expect("completion queue is empty");
            assert!(cqe.result() >= 0, "write error: {}", cqe.result());
        }

        // Read data from the file
        {
            let mut buf = [0u8; 1024];
            let read_e =
                opcode::Read::new(types::Fd(fd.as_raw_fd()), buf.as_mut_ptr(), buf.len() as _)
                    .build();

            unsafe {
                ring.submission()
                    .push(&read_e)
                    .expect("submission queue is full");
            }

            ring.submit_and_wait(1)
                .expect("Failed to submit read request to ring");
            let cqe = ring.completion().next().expect("completion queue is empty");
            assert!(cqe.result() >= 0, "read error: {}", cqe.result());

            assert_eq!(buf, [0xA; 1024]);
            fs::remove_file(file_name).unwrap();
        }
    }
}
