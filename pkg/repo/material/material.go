package material

import (
	"context"

	"github.com/scienceol/osdl/pkg/middleware/db"
	"github.com/scienceol/osdl/pkg/repo"
	"github.com/scienceol/osdl/pkg/repo/model"
)

type materialImpl struct {
	*db.Datastore
}

func NewMaterialImpl() repo.MaterialRepo {
	return &materialImpl{Datastore: db.DB()}
}

func (m *materialImpl) CreateMaterialNode(ctx context.Context, node *model.MaterialNode) error {
	return m.DBWithContext(ctx).Create(node).Error
}

func (m *materialImpl) UpdateMaterialNode(ctx context.Context, node *model.MaterialNode) error {
	return m.DBWithContext(ctx).Model(node).Updates(node).Error
}

func (m *materialImpl) GetMaterialNodes(ctx context.Context, labID int64) ([]*model.MaterialNode, error) {
	var nodes []*model.MaterialNode
	err := m.DBWithContext(ctx).Where("lab_id = ?", labID).Find(&nodes).Error
	return nodes, err
}

func (m *materialImpl) GetMaterialNodeByUUID(ctx context.Context, labID int64, uuid string) (*model.MaterialNode, error) {
	node := &model.MaterialNode{}
	err := m.DBWithContext(ctx).Where("lab_id = ? AND uuid = ?", labID, uuid).First(node).Error
	return node, err
}

func (m *materialImpl) DeleteMaterialNodes(ctx context.Context, labID int64, ids []int64) error {
	return m.DBWithContext(ctx).Where("lab_id = ? AND id IN ?", labID, ids).Delete(&model.MaterialNode{}).Error
}

func (m *materialImpl) CreateMaterialEdge(ctx context.Context, edge *model.MaterialEdge) error {
	return m.DBWithContext(ctx).Create(edge).Error
}

func (m *materialImpl) GetMaterialEdges(ctx context.Context, labID int64) ([]*model.MaterialEdge, error) {
	var edges []*model.MaterialEdge
	err := m.DBWithContext(ctx).Where("lab_id = ?", labID).Find(&edges).Error
	return edges, err
}

func (m *materialImpl) DeleteMaterialEdges(ctx context.Context, labID int64, ids []int64) error {
	return m.DBWithContext(ctx).Where("lab_id = ? AND id IN ?", labID, ids).Delete(&model.MaterialEdge{}).Error
}

func (m *materialImpl) UpdateMaterialNodeDataKey(_ context.Context, _ int64, _ string, _ string, _ any) ([]*model.MaterialNode, error) {
	// TODO: Implement material node data key update
	return nil, nil
}

func (m *materialImpl) BatchCreateMaterialNodes(ctx context.Context, nodes []*model.MaterialNode) error {
	if len(nodes) == 0 {
		return nil
	}
	return m.DBWithContext(ctx).Create(&nodes).Error
}

func (m *materialImpl) BatchCreateMaterialEdges(ctx context.Context, edges []*model.MaterialEdge) error {
	if len(edges) == 0 {
		return nil
	}
	return m.DBWithContext(ctx).Create(&edges).Error
}
