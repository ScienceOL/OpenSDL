package grpc

import (
	"fmt"
	"net"

	"github.com/scienceol/osdl/pkg/middleware/logger"
	"golang.org/x/net/context"
	ggrpc "google.golang.org/grpc"
	"google.golang.org/grpc/reflection"
)

func NewServer(ctx context.Context, port int) (*ggrpc.Server, error) {
	lis, err := net.Listen("tcp", fmt.Sprintf(":%d", port))
	if err != nil {
		return nil, fmt.Errorf("failed to listen: %w", err)
	}

	s := ggrpc.NewServer()
	reflection.Register(s)

	// TODO: Register gRPC services here
	// osdlv1.RegisterEdgeServiceServer(s, &edgeService{})
	// osdlv1.RegisterScheduleServiceServer(s, &scheduleService{})
	// osdlv1.RegisterMaterialServiceServer(s, &materialService{})
	// osdlv1.RegisterAuthServiceServer(s, &authService{})

	go func() {
		logger.Infof(ctx, "gRPC server starting on port %d", port)
		if err := s.Serve(lis); err != nil {
			logger.Errorf(ctx, "gRPC server error: %v", err)
		}
	}()

	return s, nil
}
