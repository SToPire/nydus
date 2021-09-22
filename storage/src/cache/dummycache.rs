// Copyright 2020 Ant Group. All rights reserved.
//
// SPDX-License-Identifier: Apache-2.0

//! A dummy implementation of the [BlobCacheMgr](trait.BlobCacheMgr.html) trait.
//!
//! The [DummyCacheMgr](struct.DummyCacheMgr.html) is a dummy implementation of the
//! [BlobCacheMgr](../trait.BlobCacheMgr.html) trait, which doesn't really cache any data.
//! Instead it just reads data from the backend, uncompressed it if needed and then pass on
//! the data to the clients.
//!
//! There are two possible usage mode of the [DummyCacheMgr]:
//! - Read compressed/uncompressed data from remote Registry/OSS backend but not cache the
//!   uncompressed data on local storage. The
//!   [is_chunk_cached()](../trait.BlobCache.html#tymethod.is_chunk_cached)
//!   method always return false to disable data prefetching.
//! - Read uncompressed data from local disk and no need to double cache the data.
//!   The [is_chunk_cached()](../trait.BlobCache.html#tymethod.is_chunk_cached) method always
//!   return true to enable data prefetching.
use std::io::Result;
use std::sync::Arc;

use nydus_utils::digest;
use vm_memory::VolatileSlice;

use crate::backend::{BlobBackend, BlobReader};
use crate::cache::{BlobCache, BlobCacheMgr};
use crate::device::{BlobChunkInfo, BlobInfo, BlobIoDesc, BlobPrefetchRequest};
use crate::factory::CacheConfig;
use crate::utils::{alloc_buf, copyv};
use crate::{compress, StorageError, StorageResult};

struct DummyCache {
    reader: Arc<dyn BlobReader>,
    cached: bool,
    compressor: compress::Algorithm,
    digester: digest::Algorithm,
    prefetch: bool,
    validate: bool,
}

impl BlobCache for DummyCache {
    fn blob_size(&self) -> Result<u64> {
        self.reader.blob_size().map_err(|e| eother!(e))
    }

    fn compressor(&self) -> compress::Algorithm {
        self.compressor
    }

    fn digester(&self) -> digest::Algorithm {
        self.digester
    }

    fn reader(&self) -> &dyn BlobReader {
        &*self.reader
    }

    fn need_validate(&self) -> bool {
        self.validate
    }

    fn is_chunk_ready(&self, _chunk: &dyn BlobChunkInfo) -> bool {
        self.cached
    }

    fn prefetch(
        &self,
        prefetches: &[BlobPrefetchRequest],
        bios: &[BlobIoDesc],
    ) -> StorageResult<usize> {
        if self.prefetch {
            let mut cnt = 0usize;
            for p in prefetches.iter() {
                if self
                    .reader
                    .prefetch_blob_data_range(p.offset, p.len)
                    .is_ok()
                {
                    cnt += 1;
                }
            }
            for b in bios {
                if self
                    .reader
                    .prefetch_blob_data_range(b.offset, b.size as u32)
                    .is_ok()
                {
                    cnt += 1;
                }
            }
            Ok(cnt)
        } else {
            Err(StorageError::Unsupported)
        }
    }

    fn stop_prefetch(&self) -> StorageResult<()> {
        if self.prefetch {
            // TODO: add reader.stop_prefetch_data()
            //self.reader.stop_prefetch_data();
        }

        Ok(())
    }

    fn read(&self, bios: &[BlobIoDesc], bufs: &[VolatileSlice]) -> Result<usize> {
        if bios.is_empty() {
            return Err(einval!("parameter `bios` is empty"));
        }

        let bios_len = bios.len();
        let offset = bios[0].offset;
        //let chunk = bios[0].chunkinfo.as_v5()?;
        let d_size = bios[0].chunkinfo.decompress_size() as usize;
        // Use the destination buffer to received the decompressed data if possible.
        if bufs.len() == 1 && bios_len == 1 && offset == 0 && bufs[0].len() >= d_size {
            if !bios[0].user_io {
                return Ok(0);
            }
            let buf = unsafe { std::slice::from_raw_parts_mut(bufs[0].as_ptr(), d_size) };
            return self.read_raw_chunk(&bios[0].chunkinfo, buf, None);
        }

        let mut user_size = 0;
        let mut buffer_holder: Vec<Vec<u8>> = Vec::with_capacity(bios.len());
        for bio in bios.iter() {
            if bio.user_io {
                let mut d = alloc_buf(bio.chunkinfo.decompress_size() as usize);
                self.read_raw_chunk(&bio.chunkinfo, d.as_mut_slice(), None)?;
                buffer_holder.push(d);
                user_size += bio.size;
            }
        }

        copyv(&buffer_holder, bufs, offset as usize, user_size, 0, 0)
            .map(|(n, _)| n)
            .map_err(|e| eother!(e))
    }
}

/// A dummy implementation of [BlobCacheMgr](../trait.BlobCacheMgr.html), simply reporting each
/// chunk as cached or not cached according to configuration.
///
/// The `DummyCacheMgr` is a dummy implementation of the `BlobCacheMgr`, which doesn't really cache
/// data. Instead it just reads data from the backend, uncompressed it if needed and then pass on
/// the data to the clients.
pub struct DummyCacheMgr {
    backend: Arc<dyn BlobBackend>,
    cached: bool,
    prefetch: bool,
    validate: bool,
}

impl DummyCacheMgr {
    /// Create a new instance of `DummmyCacheMgr`.
    pub fn new(
        config: CacheConfig,
        backend: Arc<dyn BlobBackend>,
        cached: bool,
        enable_prefetch: bool,
    ) -> Result<DummyCacheMgr> {
        Ok(DummyCacheMgr {
            backend,
            cached,
            validate: config.cache_validate,
            prefetch: enable_prefetch,
        })
    }
}

impl BlobCacheMgr for DummyCacheMgr {
    fn init(&self) -> Result<()> {
        Ok(())
    }

    fn destroy(&self) {
        self.backend().shutdown()
    }

    fn backend(&self) -> &(dyn BlobBackend) {
        self.backend.as_ref()
    }

    fn get_blob_cache(&self, blob_info: &Arc<BlobInfo>) -> Result<Arc<dyn BlobCache>> {
        let reader = self
            .backend
            .get_reader(blob_info.blob_id())
            .map_err(|e| eother!(e))?;

        Ok(Arc::new(DummyCache {
            reader,
            cached: self.cached,
            compressor: blob_info.compressor(),
            digester: blob_info.digester(),
            prefetch: self.prefetch,
            validate: self.validate,
        }))
    }
}
