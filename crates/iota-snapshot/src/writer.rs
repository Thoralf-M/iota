// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

#![allow(dead_code)]

use std::{
    collections::{HashMap, hash_map::Entry::Vacant},
    fs,
    fs::{File, OpenOptions},
    io::{BufWriter, Seek, SeekFrom, Write},
    num::NonZeroUsize,
    path::PathBuf,
    sync::Arc,
};

use anyhow::{Context, Result};
use byteorder::{BigEndian, ByteOrder};
use fastcrypto::hash::MultisetHash;
use futures::StreamExt;
use integer_encoding::VarInt;
use iota_config::object_storage_config::ObjectStoreConfig;
use iota_core::{
    authority::authority_store_tables::{AuthorityPerpetualTables, LiveObject},
    state_accumulator::StateAccumulator,
};
use iota_storage::{
    blob::{BLOB_ENCODING_BYTES, Blob, BlobEncoding},
    object_store::util::{copy_file, delete_recursively, path_to_filesystem},
};
use iota_types::{
    accumulator::Accumulator,
    base_types::{ObjectID, ObjectRef},
    messages_checkpoint::ECMHLiveObjectSetDigest,
};
use object_store::{DynObjectStore, path::Path};
use tokio::{
    sync::{
        mpsc,
        mpsc::{Receiver, Sender},
    },
    task::JoinHandle,
};
use tokio_stream::wrappers::ReceiverStream;
use tracing::debug;

use crate::{
    FILE_MAX_BYTES, FileCompression, FileMetadata, FileType, MAGIC_BYTES, MANIFEST_FILE_MAGIC,
    Manifest, ManifestV1, OBJECT_FILE_MAGIC, OBJECT_REF_BYTES, REFERENCE_FILE_MAGIC,
    SEQUENCE_NUM_BYTES, compute_sha3_checksum, create_file_metadata,
};

/// LiveObjectSetWriterV1 writes live object set. It creates multiple *.obj
/// files and *.ref file
struct LiveObjectSetWriterV1 {
    dir_path: PathBuf,
    bucket_num: u32,
    current_part_num: u32,
    obj_wbuf: BufWriter<File>,
    ref_wbuf: BufWriter<File>,
    object_file_size: usize,
    files: Vec<FileMetadata>,
    sender: Option<Sender<FileMetadata>>,
    file_compression: FileCompression,
}

impl LiveObjectSetWriterV1 {
    fn new(
        dir_path: PathBuf,
        bucket_num: u32,
        file_compression: FileCompression,
        sender: Sender<FileMetadata>,
    ) -> Result<Self> {
        let part_num = 1;
        let (n, obj_file) = Self::object_file(dir_path.clone(), bucket_num, part_num)?;
        let ref_file = Self::ref_file(dir_path.clone(), bucket_num, part_num)?;
        Ok(LiveObjectSetWriterV1 {
            dir_path,
            bucket_num,
            current_part_num: part_num,
            obj_wbuf: BufWriter::new(obj_file),
            ref_wbuf: BufWriter::new(ref_file),
            object_file_size: n,
            files: vec![],
            sender: Some(sender),
            file_compression,
        })
    }

    /// Writes a live object to the object file and the reference to the
    /// reference file.
    pub fn write(&mut self, object: &LiveObject) -> Result<()> {
        let object_reference = object.object_reference();
        self.write_object(object)?;
        self.write_object_ref(&object_reference)?;
        Ok(())
    }

    /// Finalizes the object and reference files and returns the FileMetadata of
    /// the files.
    pub fn done(mut self) -> Result<Vec<FileMetadata>> {
        self.finalize_obj()?;
        self.finalize_ref()?;
        self.sender = None;
        Ok(self.files.clone())
    }

    /// Creates a new object file for the provided bucket number and part
    /// number, and returns the file and the number of bytes written to it.
    fn object_file(dir_path: PathBuf, bucket_num: u32, part_num: u32) -> Result<(usize, File)> {
        let next_part_file_path = dir_path.join(format!("{bucket_num}_{part_num}.obj"));
        let next_part_file_tmp_path = dir_path.join(format!("{bucket_num}_{part_num}.obj.tmp"));
        let mut f = File::create(next_part_file_tmp_path.clone())?;
        let mut metab = [0u8; MAGIC_BYTES];
        BigEndian::write_u32(&mut metab, OBJECT_FILE_MAGIC);
        f.rewind()?;
        let n = f.write(&metab)?;
        drop(f);
        fs::rename(next_part_file_tmp_path, next_part_file_path.clone())?;
        let mut f = OpenOptions::new().append(true).open(next_part_file_path)?;
        f.seek(SeekFrom::Start(n as u64))?;
        Ok((n, f))
    }

    /// Creates a new reference file for the provided bucket number and part
    /// number, and returns the file and the number of bytes written to the
    /// file.
    fn ref_file(dir_path: PathBuf, bucket_num: u32, part_num: u32) -> Result<File> {
        let ref_path = dir_path.join(format!("{bucket_num}_{part_num}.ref"));
        let ref_tmp_path = dir_path.join(format!("{bucket_num}_{part_num}.ref.tmp"));
        let mut f = File::create(ref_tmp_path.clone())?;
        f.rewind()?;
        let mut metab = [0u8; MAGIC_BYTES];
        BigEndian::write_u32(&mut metab, REFERENCE_FILE_MAGIC);
        let n = f.write(&metab)?;
        drop(f);
        fs::rename(ref_tmp_path, ref_path.clone())?;
        let mut f = OpenOptions::new().append(true).open(ref_path)?;
        f.seek(SeekFrom::Start(n as u64))?;
        Ok(f)
    }

    /// Finalizes the object file by flushing the buffer to disk and sends its
    /// FileMetadata to the channel.
    fn finalize_obj(&mut self) -> Result<()> {
        // Flushes the buffer and sync the data to disk
        self.obj_wbuf.flush()?;
        self.obj_wbuf.get_ref().sync_data()?;
        let off = self.obj_wbuf.get_ref().stream_position()?;
        self.obj_wbuf.get_ref().set_len(off)?;
        let file_path = self
            .dir_path
            .join(format!("{}_{}.obj", self.bucket_num, self.current_part_num));
        let file_metadata = create_file_metadata(
            &file_path,
            self.file_compression,
            FileType::Object,
            self.bucket_num,
            self.current_part_num,
        )?;
        self.files.push(file_metadata.clone());
        if let Some(sender) = &self.sender {
            sender.blocking_send(file_metadata)?;
        }
        Ok(())
    }

    /// Finalizes the reference file by flushing the buffer to disk and sends
    /// its FileMetadata to the channel.
    fn finalize_ref(&mut self) -> Result<()> {
        // Flushes the buffer and sync the data to disk
        self.ref_wbuf.flush()?;
        self.ref_wbuf.get_ref().sync_data()?;
        let off = self.ref_wbuf.get_ref().stream_position()?;
        self.ref_wbuf.get_ref().set_len(off)?;
        let file_path = self
            .dir_path
            .join(format!("{}_{}.ref", self.bucket_num, self.current_part_num));
        let file_metadata = create_file_metadata(
            &file_path,
            self.file_compression,
            FileType::Reference,
            self.bucket_num,
            self.current_part_num,
        )?;
        self.files.push(file_metadata.clone());
        if let Some(sender) = &self.sender {
            sender.blocking_send(file_metadata)?;
        }
        Ok(())
    }

    /// Finalizes the object file of current partition and creates a new one for
    /// the next partition.
    fn cut(&mut self) -> Result<()> {
        self.finalize_obj()?;
        let (n, f) = Self::object_file(
            self.dir_path.clone(),
            self.bucket_num,
            self.current_part_num + 1,
        )?;
        self.object_file_size = n;
        self.obj_wbuf = BufWriter::new(f);
        Ok(())
    }

    /// Finalizes the reference file of current partition and creates a new one
    /// for the next partition.
    fn cut_reference_file(&mut self) -> Result<()> {
        self.finalize_ref()?;
        let f = Self::ref_file(
            self.dir_path.clone(),
            self.bucket_num,
            self.current_part_num + 1,
        )?;
        self.ref_wbuf = BufWriter::new(f);
        Ok(())
    }

    /// Writes a live object to the object file. Creates a new partition (new
    /// object file and reference file) if it exceeds the maximum size.
    fn write_object(&mut self, object: &LiveObject) -> Result<()> {
        let blob = Blob::encode(object, BlobEncoding::Bcs)?;
        let mut blob_size = blob.data.len().required_space();
        blob_size += BLOB_ENCODING_BYTES;
        blob_size += blob.data.len();
        let cut_new_part_file = (self.object_file_size + blob_size) > FILE_MAX_BYTES;
        if cut_new_part_file {
            self.cut()?;
            self.cut_reference_file()?;
            self.current_part_num += 1;
        }
        self.object_file_size += blob.write(&mut self.obj_wbuf)?;
        Ok(())
    }

    /// Writes an object reference to the reference file.
    fn write_object_ref(&mut self, object_ref: &ObjectRef) -> Result<()> {
        let mut buf = [0u8; OBJECT_REF_BYTES];
        buf[0..ObjectID::LENGTH].copy_from_slice(object_ref.0.as_ref());
        BigEndian::write_u64(
            &mut buf[ObjectID::LENGTH..OBJECT_REF_BYTES],
            object_ref.1.value(),
        );
        buf[ObjectID::LENGTH + SEQUENCE_NUM_BYTES..OBJECT_REF_BYTES]
            .copy_from_slice(object_ref.2.as_ref());
        self.ref_wbuf.write_all(&buf)?;
        Ok(())
    }
}

/// StateSnapshotWriterV1 writes snapshot files to a local staging dir and
/// simultaneously uploads them to a remote object store
pub struct StateSnapshotWriterV1 {
    local_staging_dir: PathBuf,
    file_compression: FileCompression,
    remote_object_store: Arc<DynObjectStore>,
    local_staging_store: Arc<DynObjectStore>,
    concurrency: usize,
}

impl StateSnapshotWriterV1 {
    pub async fn new_from_store(
        local_staging_path: &std::path::Path,
        local_staging_store: &Arc<DynObjectStore>,
        remote_object_store: &Arc<DynObjectStore>,
        file_compression: FileCompression,
        concurrency: NonZeroUsize,
    ) -> Result<Self> {
        Ok(StateSnapshotWriterV1 {
            file_compression,
            local_staging_dir: local_staging_path.to_path_buf(),
            remote_object_store: remote_object_store.clone(),
            local_staging_store: local_staging_store.clone(),
            concurrency: concurrency.get(),
        })
    }

    pub async fn new(
        local_store_config: &ObjectStoreConfig,
        remote_store_config: &ObjectStoreConfig,
        file_compression: FileCompression,
        concurrency: NonZeroUsize,
    ) -> Result<Self> {
        let remote_object_store = remote_store_config.make()?;
        let local_staging_store = local_store_config.make()?;
        let local_staging_dir = local_store_config
            .directory
            .as_ref()
            .context("No local directory specified")?
            .clone();
        Ok(StateSnapshotWriterV1 {
            local_staging_dir,
            file_compression,
            remote_object_store,
            local_staging_store,
            concurrency: concurrency.get(),
        })
    }

    /// Retrieves the system state object from the perpetual database, writes
    /// the state snapshot for the specified epoch to the local staging
    /// directory, and uploads it to the remote store.
    pub async fn write(
        self,
        epoch: u64,
        perpetual_db: Arc<AuthorityPerpetualTables>,
        root_state_hash: ECMHLiveObjectSetDigest,
    ) -> Result<()> {
        self.write_internal(epoch, perpetual_db, root_state_hash)
            .await
    }

    /// Writes the state snapshot for the provided epoch to the local staging
    /// directory and uploads it to the remote store.
    pub(crate) async fn write_internal(
        mut self,
        epoch: u64,
        perpetual_db: Arc<AuthorityPerpetualTables>,
        root_state_hash: ECMHLiveObjectSetDigest,
    ) -> Result<()> {
        self.setup_epoch_dir(epoch).await?;

        let manifest_file_path = self.epoch_dir(epoch).child("MANIFEST");
        let local_staging_dir = self.local_staging_dir.clone();
        let local_object_store = self.local_staging_store.clone();
        let remote_object_store = self.remote_object_store.clone();

        let (sender, receiver) = mpsc::channel::<FileMetadata>(1000);
        // Starts the upload loop, which listens on the receiver for FileMetadata
        let upload_handle = self.start_upload(epoch, receiver)?;
        let write_handler = tokio::task::spawn_blocking(move || {
            self.write_live_object_set(
                epoch,
                perpetual_db,
                sender,
                Self::bucket_func,
                root_state_hash,
            )
        });
        // Awaits the object and reference files to be written to the local staging
        // directory and informs the upload loop
        write_handler.await?.context(format!(
            "Failed to write state snapshot for epoch: {}",
            &epoch
        ))?;

        // Awaits the upload loop to finish
        upload_handle.await?.context(format!(
            "Failed to upload state snapshot for epoch: {}",
            &epoch
        ))?;

        // Syncs the manifest file to the remote store
        Self::sync_file_to_remote(
            local_staging_dir,
            manifest_file_path,
            local_object_store,
            remote_object_store,
        )
        .await?;
        Ok(())
    }

    /// Starts listening on the receiver for FileMetadata and uploads the files
    /// to the remote store in parallel.
    fn start_upload(
        &self,
        epoch: u64,
        receiver: Receiver<FileMetadata>,
    ) -> Result<JoinHandle<Result<Vec<()>, anyhow::Error>>> {
        let remote_object_store = self.remote_object_store.clone();
        let local_staging_store = self.local_staging_store.clone();
        let local_dir_path = self.local_staging_dir.clone();
        let epoch_dir = self.epoch_dir(epoch);
        let upload_concurrency = self.concurrency;
        let join_handle = tokio::spawn(async move {
            // Uploads the files to the remote store in parallel for each received
            // FileMetadata
            let results: Vec<Result<(), anyhow::Error>> = ReceiverStream::new(receiver)
                .map(|file_metadata| {
                    let file_path = file_metadata.file_path(&epoch_dir);
                    let remote_object_store = remote_object_store.clone();
                    let local_object_store = local_staging_store.clone();
                    let local_dir_path = local_dir_path.clone();
                    async move {
                        Self::sync_file_to_remote(
                            local_dir_path.clone(),
                            file_path.clone(),
                            local_object_store.clone(),
                            remote_object_store.clone(),
                        )
                        .await?;
                        Ok(())
                    }
                })
                .boxed()
                .buffer_unordered(upload_concurrency)
                .collect()
                .await;
            results
                .into_iter()
                .collect::<Result<Vec<()>, anyhow::Error>>()
        });
        Ok(join_handle)
    }

    /// Writes the provided live object set in the form of reference files,
    /// object files, and MANIFEST. These files are stored in the local
    /// staging directory and the FileMetadata is sent to the channel.
    fn write_live_object_set<F>(
        &mut self,
        epoch: u64,
        perpetual_db: Arc<AuthorityPerpetualTables>,
        sender: Sender<FileMetadata>,
        bucket_func: F,
        root_state_hash: ECMHLiveObjectSetDigest,
    ) -> Result<()>
    where
        F: Fn(&LiveObject) -> u32,
    {
        let mut object_writers: HashMap<u32, LiveObjectSetWriterV1> = HashMap::new();
        let local_staging_dir_path =
            path_to_filesystem(self.local_staging_dir.clone(), &self.epoch_dir(epoch))?;
        let mut acc = Accumulator::default();
        for object in perpetual_db.iter_live_object_set() {
            StateAccumulator::accumulate_live_object(&mut acc, &object);
            let bucket_num = bucket_func(&object);
            // Creates a new LiveObjectSetWriterV1 for the bucket if it does not exist
            if let Vacant(entry) = object_writers.entry(bucket_num) {
                entry.insert(LiveObjectSetWriterV1::new(
                    local_staging_dir_path.clone(),
                    bucket_num,
                    self.file_compression,
                    sender.clone(),
                )?);
            }
            let writer = object_writers
                .get_mut(&bucket_num)
                .context("Unexpected missing bucket writer")?;
            writer.write(&object)?;
        }
        assert_eq!(
            ECMHLiveObjectSetDigest::from(acc.digest()),
            root_state_hash,
            "Root state hash mismatch!"
        );
        let mut files = vec![];
        // Flushes the object and reference files to disk, informs the file channel of
        // flushed files and get the FileMetadata
        for (_, writer) in object_writers.into_iter() {
            files.extend(writer.done()?);
        }
        // Write the manifest file for the epoch(bucket)
        self.write_manifest(epoch, files)?;
        Ok(())
    }

    /// Writes the manifest file for the provided FileMetadata of an epoch and
    /// its sha3 checksum.
    fn write_manifest(&mut self, epoch: u64, file_metadata: Vec<FileMetadata>) -> Result<()> {
        let (f, manifest_file_path) = self.manifest_file(epoch)?;
        let mut wbuf = BufWriter::new(f);
        let manifest: Manifest = Manifest::V1(ManifestV1 {
            snapshot_version: 1,
            address_length: ObjectID::LENGTH as u64,
            file_metadata,
            epoch,
        });
        let serialized_manifest = bcs::to_bytes(&manifest)?;
        wbuf.write_all(&serialized_manifest)?;
        wbuf.flush()?;
        wbuf.get_ref().sync_data()?;
        // Computes the sha3 checksum of the manifest file and write it to the end of
        // the file
        let sha3_digest = compute_sha3_checksum(&manifest_file_path)?;
        wbuf.write_all(&sha3_digest)?;
        wbuf.flush()?;
        wbuf.get_ref().sync_data()?;
        let off = wbuf.get_ref().stream_position()?;
        wbuf.get_ref().set_len(off)?;
        Ok(())
    }

    /// Creates a new manifest file for the provided epoch and returns the file
    /// and the path to the file.
    fn manifest_file(&mut self, epoch: u64) -> Result<(File, PathBuf)> {
        let manifest_file_path = path_to_filesystem(
            self.local_staging_dir.clone(),
            &self.epoch_dir(epoch).child("MANIFEST"),
        )?;
        let manifest_file_tmp_path = path_to_filesystem(
            self.local_staging_dir.clone(),
            &self.epoch_dir(epoch).child("MANIFEST.tmp"),
        )?;
        let mut f = File::create(manifest_file_tmp_path.clone())?;
        let mut metab = vec![0u8; MAGIC_BYTES];
        BigEndian::write_u32(&mut metab, MANIFEST_FILE_MAGIC);
        f.rewind()?;
        f.write_all(&metab)?;
        drop(f);
        fs::rename(manifest_file_tmp_path, manifest_file_path.clone())?;
        let mut f = OpenOptions::new()
            .append(true)
            .open(manifest_file_path.clone())?;
        f.seek(SeekFrom::Start(MAGIC_BYTES as u64))?;
        Ok((f, manifest_file_path))
    }

    fn bucket_func(_object: &LiveObject) -> u32 {
        // TODO: Use the hash bucketing function used for accumulator tree if there is
        // one
        1u32
    }

    fn epoch_dir(&self, epoch: u64) -> Path {
        Path::from(format!("epoch_{}", epoch))
    }

    /// Creates a new epoch directory and a new staging directory for the epoch
    /// in the local store. Deletes the old ones if they exist.
    async fn setup_epoch_dir(&self, epoch: u64) -> Result<()> {
        let epoch_dir = self.epoch_dir(epoch);
        // Deletes remote epoch dir if it exists
        delete_recursively(
            &epoch_dir,
            &self.remote_object_store,
            NonZeroUsize::new(self.concurrency).unwrap(),
        )
        .await?;
        // Deletes local staging epoch dir if it exists
        let local_epoch_dir_path = self.local_staging_dir.join(format!("epoch_{}", epoch));
        if local_epoch_dir_path.exists() {
            fs::remove_dir_all(&local_epoch_dir_path)?;
        }
        fs::create_dir_all(&local_epoch_dir_path)?;
        Ok(())
    }

    /// Syncs a file from local store to remote store and removes the local file
    async fn sync_file_to_remote(
        local_path: PathBuf,
        path: Path,
        from: Arc<DynObjectStore>,
        to: Arc<DynObjectStore>,
    ) -> Result<()> {
        debug!("Syncing snapshot file to remote: {:?}", path);
        copy_file(&path, &path, &from, &to).await?;
        fs::remove_file(path_to_filesystem(local_path, &path)?)?;
        Ok(())
    }
}
