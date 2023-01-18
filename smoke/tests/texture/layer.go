// Copyright 2023 Nydus Developers. All rights reserved.
//
// SPDX-License-Identifier: Apache-2.0

package texture

import (
	"fmt"
	"path/filepath"
	"syscall"
	"testing"

	"github.com/dragonflyoss/image-service/smoke/tests/tool"
)

func MakeChunkDictLayer(t *testing.T, workDir string) *tool.Layer {
	layer := tool.NewLayer(t, workDir)

	// Create regular file
	layer.CreateFile(t, "chunk-dict-file-1", []byte("file-1"))
	layer.CreateFile(t, "chunk-dict-file-2", []byte("file-2"))
	layer.CreateFile(t, "chunk-dict-file-3", []byte("dir-1/file-1"))
	layer.CreateFile(t, "chunk-dict-file-4", []byte("dir-2/file-1"))
	layer.CreateFile(t, "chunk-dict-file-5", []byte("dir-1/file-2"))

	return layer
}

func MakeLowerLayer(t *testing.T, workDir string) *tool.Layer {
	layer := tool.NewLayer(t, workDir)

	// Create regular file
	layer.CreateFile(t, "file-1", []byte("file-1"))
	layer.CreateFile(t, "file-2", []byte("file-2"))

	// Create directory
	layer.CreateDir(t, "dir-1")
	layer.CreateDir(t, "dir-2/dir-1")
	layer.CreateFile(t, "dir-2/file-1", []byte("dir-2/file-1"))

	// Create hardlink
	layer.CreateFile(t, "dir-1/file-1", []byte("dir-1/file-1"))
	layer.CreateHardlink(t, "dir-1/file-1-hardlink-1", "dir-1/file-1")
	layer.CreateHardlink(t, "dir-1/file-1-hardlink-2", "dir-1/file-1")

	// Create symlink
	layer.CreateSymlink(t, "dir-1/file-1-symlink-1", "dir-1/file-1")
	layer.CreateSymlink(t, "dir-1/file-1-symlink-2", "dir-1/file-1")

	// Create special files
	layer.CreateSpecialFile(t, "char-1", syscall.S_IFCHR)
	layer.CreateSpecialFile(t, "block-1", syscall.S_IFBLK)
	layer.CreateSpecialFile(t, "fifo-1", syscall.S_IFIFO)

	layer.CreateFile(t, "dir-1/file-2", []byte("dir-1/file-2"))
	// Set file xattr (only `security.capability` xattr is supported in OCI layer)
	tool.Run(t, fmt.Sprintf("setcap CAP_NET_RAW+ep %s", filepath.Join(workDir, "dir-1/file-2")))

	return layer
}

func MakeUpperLayer(t *testing.T, workDir string) *tool.Layer {
	layer := tool.NewLayer(t, workDir)

	layer.CreateDir(t, "dir-1")
	layer.CreateFile(t, "dir-1/file-1", []byte("dir-1/upper-file-1"))
	layer.CreateWhiteout(t, "dir-1/file-2")

	layer.CreateDir(t, "dir-2")
	layer.CreateOpaque(t, "dir-2")
	layer.CreateFile(t, "dir-2/file-1", []byte("dir-2/upper-file-1"))
	// Set file xattr (only `security.capability` xattr is supported in OCI layer)
	tool.Run(t, fmt.Sprintf("setcap CAP_NET_RAW+ep %s", filepath.Join(workDir, "dir-2/file-1")))

	return layer
}
