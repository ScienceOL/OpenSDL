package services

import (
	"context"
	"encoding/json"
	"fmt"
	"time"

	r "github.com/redis/go-redis/v9"
	osdlv1 "github.com/scienceol/osdl/gen/osdl/v1"
	"github.com/scienceol/osdl/pkg/common/uuid"
	"github.com/scienceol/osdl/pkg/middleware/logger"
	"github.com/scienceol/osdl/pkg/utils"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

type EdgeService struct {
	osdlv1.UnimplementedEdgeServiceServer
	rClient *r.Client
}

func NewEdgeService(rClient *r.Client) *EdgeService {
	return &EdgeService{rClient: rClient}
}

func (s *EdgeService) GetEdgeStatus(ctx context.Context, req *osdlv1.GetEdgeStatusRequest) (*osdlv1.GetEdgeStatusResponse, error) {
	labUUID, err := uuid.FromString(req.GetLabUuid())
	if err != nil {
		return nil, status.Errorf(codes.InvalidArgument, "invalid lab_uuid: %v", err)
	}

	heartKey := utils.LabHeartName(labUUID)
	session, err := s.rClient.Get(ctx, heartKey).Result()
	if err == r.Nil {
		return &osdlv1.GetEdgeStatusResponse{
			IsOnline:      false,
			EdgeSession:   "",
			LastHeartbeat: 0,
		}, nil
	}
	if err != nil {
		logger.Errorf(ctx, "EdgeService.GetEdgeStatus Get err: %v", err)
		return nil, status.Errorf(codes.Internal, "failed to get edge status: %v", err)
	}

	ttl, _ := s.rClient.TTL(ctx, heartKey).Result()
	lastHeartbeat := time.Now().Add(-(utils.LabHeartTime + time.Second - ttl)).Unix()

	return &osdlv1.GetEdgeStatusResponse{
		IsOnline:      true,
		EdgeSession:   session,
		LastHeartbeat: lastHeartbeat,
	}, nil
}

func (s *EdgeService) StreamDeviceStatus(req *osdlv1.StreamDeviceStatusRequest, stream grpc.ServerStreamingServer[osdlv1.DeviceStatusEvent]) error {
	ctx := stream.Context()
	channel := fmt.Sprintf("osdl:device:status:%s", req.GetLabUuid())
	sub := s.rClient.Subscribe(ctx, channel)
	defer sub.Close()

	ch := sub.Channel()
	for {
		select {
		case <-ctx.Done():
			return nil
		case msg, ok := <-ch:
			if !ok {
				return nil
			}
			event := &osdlv1.DeviceStatusEvent{}
			if err := json.Unmarshal([]byte(msg.Payload), event); err != nil {
				logger.Errorf(ctx, "EdgeService.StreamDeviceStatus unmarshal err: %v", err)
				continue
			}
			if err := stream.Send(event); err != nil {
				return err
			}
		}
	}
}
