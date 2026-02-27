package grpc

import (
	"fmt"
	"net"

	osdlv1 "github.com/scienceol/osdl/gen/osdl/v1"
	materialImpl "github.com/scienceol/osdl/pkg/core/material/material"
	"github.com/scienceol/osdl/pkg/grpc/services"
	"github.com/scienceol/osdl/pkg/middleware/logger"
	"github.com/scienceol/osdl/pkg/middleware/redis"
	"golang.org/x/net/context"
	ggrpc "google.golang.org/grpc"
	"google.golang.org/grpc/reflection"
)

func NewServer(ctx context.Context, port int) (*ggrpc.Server, error) {
	lis, err := net.Listen("tcp", fmt.Sprintf(":%d", port))
	if err != nil {
		return nil, fmt.Errorf("failed to listen: %w", err)
	}

	s := ggrpc.NewServer(
		ggrpc.UnaryInterceptor(UnaryAuthInterceptor()),
		ggrpc.StreamInterceptor(StreamAuthInterceptor()),
	)
	reflection.Register(s)

	rClient := redis.GetClient()
	materialSvc := materialImpl.NewMaterial(ctx, nil)
	osdlv1.RegisterScheduleServiceServer(s, services.NewScheduleService(rClient))
	osdlv1.RegisterMaterialServiceServer(s, services.NewMaterialService(materialSvc))
	osdlv1.RegisterEdgeServiceServer(s, services.NewEdgeService(rClient))
	osdlv1.RegisterAuthServiceServer(s, services.NewAuthService())

	go func() {
		logger.Infof(ctx, "gRPC server starting on port %d", port)
		if err := s.Serve(lis); err != nil {
			logger.Errorf(ctx, "gRPC server error: %v", err)
		}
	}()

	return s, nil
}
