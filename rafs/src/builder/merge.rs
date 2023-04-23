// Copyright (C) 2022 Nydus Developers. All rights reserved.
//
// SPDX-License-Identifier: Apache-2.0

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::convert::TryFrom;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, bail, ensure, Context, Result};
use hex::FromHex;
use nydus_api::ConfigV2;
use nydus_storage::device::{BlobFeatures, BlobInfo};

use super::{
    ArtifactStorage, BlobContext, BlobManager, Bootstrap, BootstrapContext, BuildContext,
    BuildOutput, ChunkSource, ConversionType, Overlay, Tree,
};
use crate::metadata::{RafsSuper, RafsVersion};

/// Struct to generate the merged RAFS bootstrap for an image from per layer RAFS bootstraps.
///
/// A container image contains one or more layers, a RAFS bootstrap is built for each layer.
/// Those per layer bootstraps could be mounted by overlayfs to form the container rootfs.
/// To improve performance by avoiding overlayfs, an image level bootstrap is generated by
/// merging per layer bootstrap with overlayfs rules applied.
pub struct Merger {}

impl Merger {
    fn get_digest_from_list(digests: &Option<Vec<String>>, idx: usize) -> Result<Option<[u8; 32]>> {
        Ok(if let Some(digests) = &digests {
            let digest = digests
                .get(idx)
                .ok_or_else(|| anyhow!("unmatched digest index {}", idx))?;
            Some(<[u8; 32]>::from_hex(digest)?)
        } else {
            None
        })
    }

    fn get_size_from_list(sizes: &Option<Vec<u64>>, idx: usize) -> Result<Option<u64>> {
        Ok(if let Some(sizes) = &sizes {
            let size = sizes
                .get(idx)
                .ok_or_else(|| anyhow!("unmatched size index {}", idx))?;
            Some(*size)
        } else {
            None
        })
    }

    /// Overlay multiple RAFS filesystems into a merged RAFS filesystem.
    ///
    /// # Arguments
    /// - sources: contains one or more per layer bootstraps in order of lower to higher.
    /// - chunk_dict: contain the chunk dictionary used to build per layer boostrap, or None.
    #[allow(clippy::too_many_arguments)]
    pub fn merge(
        ctx: &mut BuildContext,
        parent_bootstrap_path: Option<String>,
        sources: Vec<PathBuf>,
        blob_digests: Option<Vec<String>>,
        blob_sizes: Option<Vec<u64>>,
        blob_toc_digests: Option<Vec<String>>,
        blob_toc_sizes: Option<Vec<u64>>,
        target: ArtifactStorage,
        chunk_dict: Option<PathBuf>,
        config_v2: Arc<ConfigV2>,
    ) -> Result<BuildOutput> {
        if sources.is_empty() {
            bail!("source bootstrap list is empty , at least one bootstrap is required");
        }
        if let Some(digests) = blob_digests.as_ref() {
            ensure!(
                digests.len() == sources.len(),
                "number of blob digest entries {} doesn't match number of sources {}",
                digests.len(),
                sources.len(),
            );
        }
        if let Some(sizes) = blob_sizes.as_ref() {
            ensure!(
                sizes.len() == sources.len(),
                "number of blob size entries {} doesn't match number of sources {}",
                sizes.len(),
                sources.len(),
            );
        }
        if let Some(toc_digests) = blob_toc_digests.as_ref() {
            ensure!(
                toc_digests.len() == sources.len(),
                "number of toc digest entries {} doesn't match number of sources {}",
                toc_digests.len(),
                sources.len(),
            );
        }
        if let Some(sizes) = blob_toc_sizes.as_ref() {
            ensure!(
                sizes.len() == sources.len(),
                "number of toc size entries {} doesn't match number of sources {}",
                sizes.len(),
                sources.len(),
            );
        }

        let mut tree: Option<Tree> = None;
        let mut blob_mgr = BlobManager::new(ctx.digester);
        let mut blob_idx_map = HashMap::new();
        let mut parent_layers = 0;

        // Load parent bootstrap
        if let Some(parent_bootstrap_path) = &parent_bootstrap_path {
            let (rs, _) =
                RafsSuper::load_from_file(parent_bootstrap_path, config_v2.clone(), false)
                    .context(format!("load parent bootstrap {:?}", parent_bootstrap_path))?;
            let blobs = rs.superblock.get_blob_infos();
            for blob in &blobs {
                let blob_ctx = BlobContext::from(ctx, &blob, ChunkSource::Parent)?;
                blob_idx_map.insert(blob_ctx.blob_id.clone(), blob_mgr.len());
                blob_mgr.add_blob(blob_ctx);
            }
            parent_layers = blobs.len();
            tree = Some(Tree::from_bootstrap(&rs, &mut ())?);
        }

        // Get the blobs come from chunk dictionary.
        let mut chunk_dict_blobs = HashSet::new();
        let mut config = None;
        if let Some(chunk_dict_path) = &chunk_dict {
            let (rs, _) = RafsSuper::load_from_file(chunk_dict_path, config_v2.clone(), false)
                .context(format!("load chunk dict bootstrap {:?}", chunk_dict_path))?;
            config = Some(rs.meta.get_config());
            for blob in rs.superblock.get_blob_infos() {
                chunk_dict_blobs.insert(blob.blob_id().to_string());
            }
        }

        let mut fs_version = RafsVersion::V6;
        let mut chunk_size = None;

        for (layer_idx, bootstrap_path) in sources.iter().enumerate() {
            let (rs, _) = RafsSuper::load_from_file(bootstrap_path, config_v2.clone(), false)
                .context(format!("load bootstrap {:?}", bootstrap_path))?;
            config
                .get_or_insert_with(|| rs.meta.get_config())
                .check_compatibility(&rs.meta)?;
            fs_version = RafsVersion::try_from(rs.meta.version)
                .context("failed to get RAFS version number")?;
            ctx.compressor = rs.meta.get_compressor();
            ctx.digester = rs.meta.get_digester();
            ctx.explicit_uidgid = rs.meta.explicit_uidgid();
            if config.as_ref().unwrap().is_tarfs_mode {
                ctx.conversion_type = ConversionType::TarToTarfs;
                ctx.blob_features |= BlobFeatures::TARFS;
            }

            let mut parent_blob_added = false;
            let blobs = &rs.superblock.get_blob_infos();
            for blob in blobs {
                let mut blob_ctx = BlobContext::from(ctx, &blob, ChunkSource::Parent)?;
                if let Some(chunk_size) = chunk_size {
                    ensure!(
                        chunk_size == blob_ctx.chunk_size,
                        "can not merge bootstraps with inconsistent chunk size, current bootstrap {:?} with chunk size {:x}, expected {:x}",
                        bootstrap_path,
                        blob_ctx.chunk_size,
                        chunk_size,
                    );
                } else {
                    chunk_size = Some(blob_ctx.chunk_size);
                }
                if !chunk_dict_blobs.contains(&blob.blob_id()) {
                    // It is assumed that the `nydus-image create` at each layer and `nydus-image merge` commands
                    // use the same chunk dict bootstrap. So the parent bootstrap includes multiple blobs, but
                    // only at most one new blob, the other blobs should be from the chunk dict image.
                    if parent_blob_added {
                        bail!("invalid per layer bootstrap, having multiple associated data blobs");
                    }
                    parent_blob_added = true;

                    if ctx.configuration.internal.blob_accessible()
                        || ctx.conversion_type == ConversionType::TarToTarfs
                    {
                        // `blob.blob_id()` should have been fixed when loading the bootstrap.
                        blob_ctx.blob_id = blob.blob_id();
                    } else {
                        // The blob id (blob sha256 hash) in parent bootstrap is invalid for nydusd
                        // runtime, should change it to the hash of whole tar blob.
                        blob_ctx.blob_id = BlobInfo::get_blob_id_from_meta_path(bootstrap_path)?;
                    }
                    if let Some(digest) = Self::get_digest_from_list(&blob_digests, layer_idx)? {
                        if blob.has_feature(BlobFeatures::SEPARATE) {
                            blob_ctx.blob_meta_digest = digest;
                        } else {
                            blob_ctx.blob_id = hex::encode(digest);
                        }
                    }
                    if let Some(size) = Self::get_size_from_list(&blob_sizes, layer_idx)? {
                        if blob.has_feature(BlobFeatures::SEPARATE) {
                            blob_ctx.blob_meta_size = size;
                        } else {
                            blob_ctx.compressed_blob_size = size;
                        }
                    }
                    if let Some(digest) = Self::get_digest_from_list(&blob_toc_digests, layer_idx)?
                    {
                        blob_ctx.blob_toc_digest = digest;
                    }
                    if let Some(size) = Self::get_size_from_list(&blob_toc_sizes, layer_idx)? {
                        blob_ctx.blob_toc_size = size as u32;
                    }
                }

                if let Entry::Vacant(e) = blob_idx_map.entry(blob.blob_id()) {
                    e.insert(blob_mgr.len());
                    blob_mgr.add_blob(blob_ctx);
                }
            }

            let upper = Tree::from_bootstrap(&rs, &mut ())?;
            upper.walk_bfs(true, &mut |n| {
                let mut node = n.lock_node();
                for chunk in &mut node.chunks {
                    let origin_blob_index = chunk.inner.blob_index() as usize;
                    let blob_ctx = blobs[origin_blob_index].as_ref();
                    if let Some(blob_index) = blob_idx_map.get(&blob_ctx.blob_id()) {
                        // Set the blob index of chunk to real index in blob table of final bootstrap.
                        chunk.set_blob_index(*blob_index as u32);
                    }
                }
                // Set node's layer index to distinguish same inode number (from bootstrap)
                // between different layers.
                let idx = u16::try_from(layer_idx).context(format!(
                    "too many layers {}, limited to {}",
                    layer_idx,
                    u16::MAX
                ))?;
                if parent_layers + idx as usize > u16::MAX as usize {
                    bail!("too many layers {}, limited to {}", layer_idx, u16::MAX);
                }
                node.layer_idx = idx + parent_layers as u16;
                node.overlay = Overlay::UpperAddition;
                Ok(())
            })?;

            if let Some(tree) = &mut tree {
                tree.merge_overaly(ctx, upper)?;
            } else {
                tree = Some(upper);
            }
        }

        if ctx.conversion_type == ConversionType::TarToTarfs {
            if parent_layers > 0 {
                bail!("merging RAFS in TARFS mode conflicts with `--parent-bootstrap`");
            }
            if !chunk_dict_blobs.is_empty() {
                bail!("merging RAFS in TARFS mode conflicts with `--chunk-dict`");
            }
        }

        // Safe to unwrap because there is at least one source bootstrap.
        let tree = tree.unwrap();
        ctx.fs_version = fs_version;
        if let Some(chunk_size) = chunk_size {
            ctx.chunk_size = chunk_size;
        }

        let mut bootstrap_ctx = BootstrapContext::new(Some(target.clone()), false)?;
        let mut bootstrap = Bootstrap::new(tree)?;
        bootstrap.build(ctx, &mut bootstrap_ctx)?;
        let blob_table = blob_mgr.to_blob_table(ctx)?;
        let mut bootstrap_storage = Some(target.clone());
        bootstrap
            .dump(ctx, &mut bootstrap_storage, &mut bootstrap_ctx, &blob_table)
            .context(format!("dump bootstrap to {:?}", target.display()))?;
        BuildOutput::new(&blob_mgr, &bootstrap_storage)
    }
}
