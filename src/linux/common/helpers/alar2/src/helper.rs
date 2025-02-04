use crate::constants;
use crate::distro::{Distro, EfiPartT, EfiPartition};
use crate::mount;
use chrono::prelude::Utc;
use cmd_lib::*;
use std::process::Stdio;
use std::{fs, process};

pub fn log_info(msg: &str) {
    println!("[Info {}] {}", Utc::now(), msg);
}

#[allow(dead_code)]
pub fn log_output(msg: &str) {
    println!("[Output {}] {}", Utc::now(), msg);
}

#[allow(dead_code)]
pub fn log_warning(msg: &str) {
    println!("[Warning {}] {}", Utc::now(), msg);
}

pub fn log_error(msg: &str) {
    println!("[Error {}] {}", Utc::now(), msg);
}

pub fn log_debug(msg: &str) {
    println!("[Debug {}] {}", Utc::now(), msg);
}

pub(crate) fn read_link(path: &str) -> String {
    match fs::canonicalize(&path) {
        Ok(value) => return format!("{}", value.display()),
        Err(e) => {
            log_error(&e.to_string());
            panic!("readlink did fail")
        }
    }
}

pub(crate) fn part_info_helper(sgdiskinfo: &str) -> String {
    let part_number = cut(sgdiskinfo, " ", 0).trim_end().parse::<u8>().unwrap();
    let path = format!("{}{}", read_link(constants::RESCUE_DISK), part_number);
    let run_output = run_fun!(blkid -o export $path);
    match run_output {
        Ok(value) => value,
        Err(_) => "ERROR".to_string(),
    }
}

pub(crate) fn cut<'a>(source: &'a str, delimiter: &str, field: usize) -> &'a str {
    match source.split(delimiter).nth(field) {
        Some(value) => value,
        None => {
            log_error("String not found. FATAL! ERROR NOT RECOVERABLE");
            panic!("Error in function cut");
        }
    }
}

pub(crate) fn get_partition_number_detail(sgdiskinfo: &str) -> u8 {
    cut(sgdiskinfo, " ", 0).parse::<u8>().unwrap()
}

pub(crate) fn get_partition_filesystem_detail(sgdiskinfo: &str) -> String {
    let info = part_info_helper(sgdiskinfo);
    let mut lines = vec![];
    let mut fs_return = "".to_string();
    for i in info.lines() {
        lines.push(i);
    }
    lines.retain(|x| x.starts_with("TYPE="));
    if let Some(fs) = lines[0].to_string().strip_prefix("TYPE=") {
        fs_return = fs.to_string();
    }
    fs_return
}

pub(crate) fn get_pretty_name(path: &str) -> String {
    let mut pretty_name: String = "".to_string();
    if let Ok(name) = run_fun!(grep -s PRETTY_NAME $path) {
        pretty_name = cut(&name, "=", 1).to_string();
    } else if let Ok(value) = fs::read_to_string(constants::REDHAT_RELEASE) {
        pretty_name = value;
    }
    log_info(format!("Pretty Name is : {}", &pretty_name).as_str());
    pretty_name
}

pub(crate) fn get_ade_mounpoint(source: &str) -> String {
    let mut mountpoint = "".to_string();
    if let Ok(path) = cmd_lib::run_fun!(cat /proc/mounts | grep $source | cut -d" " -f2) {
        mountpoint = path;
    }
    log_info(format!("unmounted: {}", &mountpoint).as_str());
    mountpoint
}

pub(crate) fn fsck_partition(partition_path: &str, partition_filesystem: &str) {
    // Need to handel the condition if no filesystem is available
    // This can happen if we have a LVM partition
    if partition_filesystem.is_empty() {
        return;
    }

    //let mut result: result::Result<String, io::Error> = Err(io::Error::new(io::ErrorKind::Other, "none")); // run_cmd returns "type CmdResult = Result<(), Error>;"
    let mut exit_code = Some(0i32);

    match partition_filesystem {
        "xfs" => {
            log_info(format!("fsck for XFS on {}", partition_path).as_str());
            if let Err(e) = mount::mkdir_assert() {
                panic!("Creating assert directory is not possible : '{}'. ALAR is not able to proceed further",e);
            }

            // In case the filesystem has valuable metadata changes in a log which needs to
            // be replayed.  Mount the filesystem to replay the log, and unmount it before
            // re-running xfs_repair
            mount::mount_path_assert(partition_path);
            mount::umount(constants::ASSERT_PATH);

            if let Ok(stat) = process::Command::new("xfs_repair")
                .arg(&partition_path)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
            {
                exit_code = stat.code();
            }
        }
        "fat16" => {
            log_info("fsck for fat16/vfat");
            if let Ok(stat) = process::Command::new("fsck.vfat")
                .args(&["-p", partition_path])
                .status()
            {
                exit_code = stat.code();
            }
        }
        _ => {
            log_info(format!("fsck for {}", partition_filesystem).as_str());
            if let Ok(stat) = process::Command::new(format!("fsck.{}", partition_filesystem))
                .args(&["-p", partition_path])
                .status()
            {
                exit_code = stat.code();
            }
        }
    }

    match exit_code {
        // error 4 is returned by fsck.ext4 only
        Some(_code @ 4) => {
            log_error(
                format!(
                    "Partition {} can not be repaired in auto mode",
                    &partition_path
                )
                .as_str(),
            );
            log_error("Aborting ALAR");
            process::exit(1);
        }
        // xfs_repair -n returns 1 if the fs is corrupted.
        // Also fsck may raise this error but we ignore it as even a normal recover is raising it. FALSE-NEGATIVE
        Some(_code @ 1) if partition_filesystem == "xfs" => {
            log_error("A general error occured while trying to recover the device ${root_rescue}.");
            log_error("Aborting ALAR");
            process::exit(1);
        }
        None => {
            panic!(
                "fsck operation terminated by signal error. ALAR is not able to proceed further!"
            );
        }

        // Any other error state is not of interest for us
        _ => {}
    }

    log_info("File system check finished");
}

pub(crate) fn set_efi_part_number_and_fs(distro: &mut Distro, partition: &str) {
    let mut new_efi_part = EfiPartT::new();
    if let EfiPartT::EfiPart(EfiPartition {
        efi_part_number: ref mut ref_to_number,
        efi_part_fs: ref mut ref_to_efi_part_fs,
        efi_part_path: _,
    }) = new_efi_part
    {
        *ref_to_efi_part_fs = get_partition_filesystem_detail(partition);
        *ref_to_number = get_partition_number_detail(partition);
    }
    distro.efi_part = new_efi_part;
}

pub(crate) fn set_efi_part_path(distro: &mut Distro) {
    // set_efi_part_path has to be used only after set_efi_part_number_and_fs has been called
    let part_number = get_efi_part_number(distro);
    if let EfiPartT::EfiPart(EfiPartition {
        efi_part_number: _,
        efi_part_fs: _,
        efi_part_path: ref mut ref_to_efi_part_path,
    }) = distro.efi_part
    {
        *ref_to_efi_part_path = format!("{}{}", read_link(constants::RESCUE_DISK), part_number);
    }
}

pub(crate) fn get_efi_part_path(distro: &Distro) -> String {
    let mut path: String = String::from("");
    if let EfiPartT::EfiPart(EfiPartition {
        efi_part_number: _,
        efi_part_fs: _,
        efi_part_path: ref ref_to_efi_part_path,
    }) = distro.efi_part
    {
        path = ref_to_efi_part_path.to_string();
    }
    path
}

pub(crate) fn has_efi_part(distro: &Distro) -> bool {
    match distro.efi_part {
        EfiPartT::NoEFI => false,
        EfiPartT::EfiPart(_) => true,
    }
}

pub(crate) fn get_efi_part_fs(distro: &Distro) -> String {
    let mut fs: String = String::from("");
    if let EfiPartT::EfiPart(EfiPartition {
        efi_part_number: _,
        efi_part_fs: ref ref_to_efi_part_fs,
        efi_part_path: _,
    }) = distro.efi_part
    {
        fs = ref_to_efi_part_fs.to_string();
    }
    fs
}

fn get_efi_part_number(distro: &Distro) -> u8 {
    let mut number: u8 = 0;
    if let EfiPartT::EfiPart(EfiPartition {
        efi_part_number: internal_number,
        efi_part_fs: _,
        efi_part_path: _,
    }) = distro.efi_part
    {
        number = internal_number;
    }
    number
}
