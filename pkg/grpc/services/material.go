package services

import (
	"context"
	"encoding/json"

	osdlv1 "github.com/scienceol/osdl/gen/osdl/v1"
	"github.com/scienceol/osdl/pkg/common/uuid"
	"github.com/scienceol/osdl/pkg/core/material"
	"github.com/scienceol/osdl/pkg/middleware/logger"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

type MaterialService struct {
	osdlv1.UnimplementedMaterialServiceServer
	materialSvc material.Service
}

func NewMaterialService(materialSvc material.Service) *MaterialService {
	return &MaterialService{materialSvc: materialSvc}
}

func (s *MaterialService) EdgeCreateMaterial(ctx context.Context, req *osdlv1.EdgeCreateMaterialRequest) (*osdlv1.EdgeCreateMaterialResponse, error) {
	createReq := &material.CreateMaterialReq{}
	if err := json.Unmarshal(req.GetData(), createReq); err != nil {
		return nil, status.Errorf(codes.InvalidArgument, "invalid data: %v", err)
	}

	items, err := s.materialSvc.EdgeCreateMaterial(ctx, createReq)
	if err != nil {
		logger.Errorf(ctx, "MaterialService.EdgeCreateMaterial err: %v", err)
		return nil, status.Errorf(codes.Internal, "create material failed: %v", err)
	}

	resp := &osdlv1.EdgeCreateMaterialResponse{}
	for _, item := range items {
		resp.Items = append(resp.Items, &osdlv1.MaterialItem{
			Uuid: item.CloudUUID.String(),
			Name: item.Name,
		})
	}
	return resp, nil
}

func (s *MaterialService) EdgeUpsertMaterial(ctx context.Context, req *osdlv1.EdgeUpsertMaterialRequest) (*osdlv1.EdgeUpsertMaterialResponse, error) {
	upsertReq := &material.UpsertMaterialReq{}
	if err := json.Unmarshal(req.GetData(), upsertReq); err != nil {
		return nil, status.Errorf(codes.InvalidArgument, "invalid data: %v", err)
	}

	items, err := s.materialSvc.EdgeUpsertMaterial(ctx, upsertReq)
	if err != nil {
		logger.Errorf(ctx, "MaterialService.EdgeUpsertMaterial err: %v", err)
		return nil, status.Errorf(codes.Internal, "upsert material failed: %v", err)
	}

	resp := &osdlv1.EdgeUpsertMaterialResponse{}
	for _, item := range items {
		resp.Items = append(resp.Items, &osdlv1.MaterialItem{
			Uuid: item.CloudUUID.String(),
			Name: item.Name,
		})
	}
	return resp, nil
}

func (s *MaterialService) EdgeCreateEdge(ctx context.Context, req *osdlv1.EdgeCreateEdgeRequest) (*osdlv1.EdgeCreateEdgeResponse, error) {
	edgeReq := &material.CreateMaterialEdgeReq{}
	if err := json.Unmarshal(req.GetData(), edgeReq); err != nil {
		return nil, status.Errorf(codes.InvalidArgument, "invalid data: %v", err)
	}

	if err := s.materialSvc.EdgeCreateEdge(ctx, edgeReq); err != nil {
		logger.Errorf(ctx, "MaterialService.EdgeCreateEdge err: %v", err)
		return nil, status.Errorf(codes.Internal, "create edge failed: %v", err)
	}

	return &osdlv1.EdgeCreateEdgeResponse{Success: true}, nil
}

func (s *MaterialService) QueryMaterial(ctx context.Context, req *osdlv1.QueryMaterialRequest) (*osdlv1.QueryMaterialResponse, error) {
	uuids := make([]uuid.UUID, 0, len(req.GetUuids()))
	for _, u := range req.GetUuids() {
		parsed, err := uuid.FromString(u)
		if err != nil {
			return nil, status.Errorf(codes.InvalidArgument, "invalid uuid %s: %v", u, err)
		}
		uuids = append(uuids, parsed)
	}

	queryReq := &material.MaterialQueryReq{UUIDS: uuids}
	result, err := s.materialSvc.EdgeQueryMaterial(ctx, queryReq)
	if err != nil {
		logger.Errorf(ctx, "MaterialService.QueryMaterial err: %v", err)
		return nil, status.Errorf(codes.Internal, "query material failed: %v", err)
	}

	data, _ := json.Marshal(result)
	return &osdlv1.QueryMaterialResponse{Data: data}, nil
}

func (s *MaterialService) DownloadMaterial(ctx context.Context, _ *osdlv1.DownloadMaterialRequest) (*osdlv1.DownloadMaterialResponse, error) {
	result, err := s.materialSvc.EdgeDownloadMaterial(ctx)
	if err != nil {
		logger.Errorf(ctx, "MaterialService.DownloadMaterial err: %v", err)
		return nil, status.Errorf(codes.Internal, "download material failed: %v", err)
	}

	data, _ := json.Marshal(result)
	return &osdlv1.DownloadMaterialResponse{Data: data}, nil
}
