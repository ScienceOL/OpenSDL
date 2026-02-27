package material

import (
	"context"

	"github.com/scienceol/osdl/pkg/common/code"
	"github.com/scienceol/osdl/pkg/core/material"
	"github.com/scienceol/osdl/pkg/middleware/auth"
	"github.com/scienceol/osdl/pkg/middleware/logger"
)

func (m *materialImpl) EdgeCreateMaterial(ctx context.Context, req *material.CreateMaterialReq) ([]*material.CreateMaterialResp, error) {
	labUser := auth.GetCurrentUser(ctx)
	if labUser == nil {
		return nil, code.UnLogin
	}
	if len(req.Nodes) == 0 {
		return nil, nil
	}
	// TODO: implement full edge create material with node hierarchy
	logger.Infof(ctx, "EdgeCreateMaterial nodes count: %d", len(req.Nodes))
	return nil, nil
}

func (m *materialImpl) EdgeUpsertMaterial(ctx context.Context, req *material.UpsertMaterialReq) ([]*material.UpsertMaterialResp, error) {
	if len(req.Nodes) == 0 {
		return nil, code.ParamErr
	}
	labUser := auth.GetCurrentUser(ctx)
	if labUser == nil {
		return nil, code.UnLogin
	}
	// TODO: implement full edge upsert material with DAG hierarchy and delta sync
	logger.Infof(ctx, "EdgeUpsertMaterial nodes count: %d", len(req.Nodes))
	return nil, nil
}

func (m *materialImpl) EdgeCreateEdge(ctx context.Context, req *material.CreateMaterialEdgeReq) error {
	labUser := auth.GetCurrentUser(ctx)
	if labUser == nil {
		return code.UnLogin
	}
	// TODO: implement edge create with handle resolution
	logger.Infof(ctx, "EdgeCreateEdge edges count: %d", len(req.Edges))
	return nil
}

func (m *materialImpl) EdgeQueryMaterial(ctx context.Context, req *material.MaterialQueryReq) (*material.MaterialQueryResp, error) {
	labUser := auth.GetCurrentUser(ctx)
	if labUser == nil {
		return nil, code.UnLogin
	}
	if len(req.UUIDS) == 0 {
		return nil, nil
	}
	// TODO: implement full material query with descendants support
	logger.Infof(ctx, "EdgeQueryMaterial uuids count: %d", len(req.UUIDS))
	return &material.MaterialQueryResp{}, nil
}

func (m *materialImpl) EdgeDownloadMaterial(ctx context.Context) (*material.DownloadMaterialResp, error) {
	labUser := auth.GetCurrentUser(ctx)
	if labUser == nil {
		return nil, code.UnLogin
	}
	// TODO: implement edge download material
	return &material.DownloadMaterialResp{}, nil
}
