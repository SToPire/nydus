package main

import (
	"context"

	"github.com/dragonflyoss/image-service/contrib/nydusify/pkg/converter"
	"github.com/dragonflyoss/image-service/contrib/nydusify/pkg/provider"
)

func main() {
	// Configurable parameters for converter
	workDir := "./tmp"
	nydusImagePath := "/path/to/nydus-image"
	source := "localhost:5000/ubuntu:latest"
	target := "localhost:5000/ubuntu:latest-nydus"
	// Set to empty if no authorization be required
	auth := "<base64_encoded_auth>"
	// Set to false if using https registry
	insecure := true

	// Logger outputs Nydus image conversion progress
	logger, err := provider.DefaultLogger()
	if err != nil {
		panic(err)
	}

	// Create remote with auth string for registry communication
	sourceRemote, err := provider.DefaultRemoteWithAuth(source, insecure, auth)
	if err != nil {
		panic(err)
	}
	// Or we can create with docker config
	// sourceRemote, err := provider.DefaultRemote(source, insecure)
	// if err != nil {
	// 	panic(err)
	// }
	targetRemote, err := provider.DefaultRemoteWithAuth(target, insecure, auth)
	if err != nil {
		panic(err)
	}

	opt := converter.Opt{
		Logger:         logger,
		TargetPlatform: "linux/amd64",
		SourceRemote:   sourceRemote,
		TargetRemote:   targetRemote,

		WorkDir:          workDir,
		PrefetchPatterns: "/",
		NydusImagePath:   nydusImagePath,
		MultiPlatform:    false,
		DockerV2Format:   true,
	}

	cvt, err := converter.New(opt)
	if err != nil {
		panic(err)
	}

	if err := cvt.Convert(context.Background()); err != nil {
		panic(err)
	}
}
