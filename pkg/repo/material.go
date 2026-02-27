package repo

import (
	"context"

	"github.com/scienceol/osdl/pkg/repo/model"
)

type MaterialRepo interface {
	CreateMaterialNode(ctx context.Context, node *model.MaterialNode) error
	UpdateMaterialNode(ctx context.Context, node *model.MaterialNode) error
	GetMaterialNodes(ctx context.Context, labID int64) ([]*model.MaterialNode, error)
	GetMaterialNodeByUUID(ctx context.Context, labID int64, uuid string) (*model.MaterialNode, error)
	DeleteMaterialNodes(ctx context.Context, labID int64, ids []int64) error
	CreateMaterialEdge(ctx context.Context, edge *model.MaterialEdge) error
	GetMaterialEdges(ctx context.Context, labID int64) ([]*model.MaterialEdge, error)
	DeleteMaterialEdges(ctx context.Context, labID int64, ids []int64) error
	UpdateMaterialNodeDataKey(ctx context.Context, labID int64, deviceID string, propertyName string, status any) ([]*model.MaterialNode, error)
	BatchCreateMaterialNodes(ctx context.Context, nodes []*model.MaterialNode) error
	BatchCreateMaterialEdges(ctx context.Context, edges []*model.MaterialEdge) error
}
