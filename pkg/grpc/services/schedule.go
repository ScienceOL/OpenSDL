package services

import (
	"context"
	"encoding/json"
	"fmt"

	r "github.com/redis/go-redis/v9"
	osdlv1 "github.com/scienceol/osdl/gen/osdl/v1"
	"github.com/scienceol/osdl/pkg/common/uuid"
	"github.com/scienceol/osdl/pkg/core/schedule/lab"
	"github.com/scienceol/osdl/pkg/middleware/logger"
	"github.com/scienceol/osdl/pkg/utils"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

type ScheduleService struct {
	osdlv1.UnimplementedScheduleServiceServer
	rClient *r.Client
}

func NewScheduleService(rClient *r.Client) *ScheduleService {
	return &ScheduleService{rClient: rClient}
}

func (s *ScheduleService) StartWorkflow(ctx context.Context, req *osdlv1.StartWorkflowRequest) (*osdlv1.StartWorkflowResponse, error) {
	labUUID, err := uuid.FromString(req.GetLabUuid())
	if err != nil {
		return nil, status.Errorf(codes.InvalidArgument, "invalid lab_uuid: %v", err)
	}

	taskUUID := uuid.NewV4()
	msg := &lab.ApiData[map[string]string]{
		ApiMsg: lab.ApiMsg{Action: lab.StartWorkflow},
		Data: map[string]string{
			"workflow_uuid": req.GetWorkflowUuid(),
			"user_id":       req.GetUserId(),
			"task_uuid":     taskUUID.String(),
		},
	}

	data, _ := json.Marshal(msg)
	if err := s.rClient.LPush(ctx, utils.LabTaskName(labUUID), string(data)).Err(); err != nil {
		logger.Errorf(ctx, "ScheduleService.StartWorkflow LPush err: %v", err)
		return nil, status.Errorf(codes.Internal, "failed to enqueue workflow: %v", err)
	}

	return &osdlv1.StartWorkflowResponse{TaskUuid: taskUUID.String()}, nil
}

func (s *ScheduleService) StartNotebook(ctx context.Context, req *osdlv1.StartNotebookRequest) (*osdlv1.StartNotebookResponse, error) {
	labUUID, err := uuid.FromString(req.GetLabUuid())
	if err != nil {
		return nil, status.Errorf(codes.InvalidArgument, "invalid lab_uuid: %v", err)
	}

	taskUUID := uuid.NewV4()
	msg := &lab.ApiData[map[string]string]{
		ApiMsg: lab.ApiMsg{Action: lab.StartNotebook},
		Data: map[string]string{
			"notebook_uuid": req.GetNotebookUuid(),
			"user_id":       req.GetUserId(),
			"task_uuid":     taskUUID.String(),
		},
	}

	data, _ := json.Marshal(msg)
	if err := s.rClient.LPush(ctx, utils.LabTaskName(labUUID), string(data)).Err(); err != nil {
		logger.Errorf(ctx, "ScheduleService.StartNotebook LPush err: %v", err)
		return nil, status.Errorf(codes.Internal, "failed to enqueue notebook: %v", err)
	}

	return &osdlv1.StartNotebookResponse{TaskUuid: taskUUID.String()}, nil
}

func (s *ScheduleService) StartAction(ctx context.Context, req *osdlv1.StartActionRequest) (*osdlv1.StartActionResponse, error) {
	labUUID, err := uuid.FromString(req.GetLabUuid())
	if err != nil {
		return nil, status.Errorf(codes.InvalidArgument, "invalid lab_uuid: %v", err)
	}

	taskUUID := uuid.NewV4()
	msg := &lab.ApiControlData[map[string]any]{
		ApiControlMsg: lab.ApiControlMsg{Action: lab.StartAction},
		Data: map[string]any{
			"device_id":   req.GetDeviceId(),
			"action":      req.GetAction(),
			"action_type": req.GetActionType(),
			"param":       req.GetParam(),
			"task_uuid":   taskUUID.String(),
		},
	}

	data, _ := json.Marshal(msg)
	if err := s.rClient.LPush(ctx, utils.LabControlName(labUUID), string(data)).Err(); err != nil {
		logger.Errorf(ctx, "ScheduleService.StartAction LPush err: %v", err)
		return nil, status.Errorf(codes.Internal, "failed to enqueue action: %v", err)
	}

	return &osdlv1.StartActionResponse{TaskUuid: taskUUID.String()}, nil
}

func (s *ScheduleService) StopJob(ctx context.Context, req *osdlv1.StopJobRequest) (*osdlv1.StopJobResponse, error) {
	taskUUID, err := uuid.FromString(req.GetTaskUuid())
	if err != nil {
		return nil, status.Errorf(codes.InvalidArgument, "invalid task_uuid: %v", err)
	}

	// Publish stop command to the control channel
	// The task_uuid is used to identify which lab's control queue to push to.
	// Since we don't have lab_uuid in the StopJob request, we broadcast via pub/sub.
	stopMsg := &lab.ApiControlData[lab.StopJobReq]{
		ApiControlMsg: lab.ApiControlMsg{Action: lab.StopJob},
		Data: lab.StopJobReq{
			UUID:   taskUUID,
			UserID: req.GetUserId(),
		},
	}

	data, _ := json.Marshal(stopMsg)
	channel := fmt.Sprintf("osdl:job:stop:%s", req.GetTaskUuid())
	if err := s.rClient.Publish(ctx, channel, string(data)).Err(); err != nil {
		logger.Errorf(ctx, "ScheduleService.StopJob publish err: %v", err)
		return nil, status.Errorf(codes.Internal, "failed to publish stop: %v", err)
	}

	return &osdlv1.StopJobResponse{Success: true}, nil
}

func (s *ScheduleService) StreamJobStatus(req *osdlv1.StreamJobStatusRequest, stream grpc.ServerStreamingServer[osdlv1.JobStatusEvent]) error {
	ctx := stream.Context()
	channel := fmt.Sprintf("osdl:job:status:%s", req.GetTaskUuid())
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
			event := &osdlv1.JobStatusEvent{}
			if err := json.Unmarshal([]byte(msg.Payload), event); err != nil {
				logger.Errorf(ctx, "ScheduleService.StreamJobStatus unmarshal err: %v", err)
				continue
			}
			if err := stream.Send(event); err != nil {
				return err
			}
		}
	}
}
