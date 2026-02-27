package material

import (
	"context"
	"encoding/json"

	"github.com/olahol/melody"
	r "github.com/redis/go-redis/v9"
	"github.com/scienceol/osdl/pkg/common"
	"github.com/scienceol/osdl/pkg/common/code"
	"github.com/scienceol/osdl/pkg/core/material"
	"github.com/scienceol/osdl/pkg/core/notify"
	"github.com/scienceol/osdl/pkg/core/notify/events"
	"github.com/scienceol/osdl/pkg/middleware/auth"
	"github.com/scienceol/osdl/pkg/middleware/logger"
	"github.com/scienceol/osdl/pkg/middleware/redis"
	"github.com/scienceol/osdl/pkg/repo"
	eStore "github.com/scienceol/osdl/pkg/repo/environment"
	mStore "github.com/scienceol/osdl/pkg/repo/material"
)

type materialImpl struct {
	envStore      repo.LaboratoryRepo
	materialStore repo.MaterialRepo
	wsClient      *melody.Melody
	msgCenter     notify.MsgCenter
	rClient       *r.Client
}

func NewMaterial(ctx context.Context, wsClient *melody.Melody) material.Service {
	m := &materialImpl{
		envStore:      eStore.New(),
		materialStore: mStore.NewMaterialImpl(),
		wsClient:      wsClient,
		msgCenter:     events.NewEvents(),
		rClient:       redis.GetClient(),
	}
	if err := events.NewEvents().Registry(ctx, notify.MaterialModify, m.OnMaterialNotify); err != nil {
		logger.Errorf(ctx, "Registry MaterialModify fail err: %+v", err)
	}
	return m
}

func (m *materialImpl) CreateMaterial(ctx context.Context, req *material.GraphNodeReq) error {
	labUser := auth.GetCurrentUser(ctx)
	if labUser == nil {
		return code.UnLogin
	}
	// TODO: implement full material creation with node/edge persistence
	return nil
}

func (m *materialImpl) SaveMaterial(_ context.Context, _ *material.SaveGrapReq) error {
	// TODO: implement save material graph
	return nil
}

func (m *materialImpl) LabMaterial(ctx context.Context, _ *material.MaterialReq) ([]*material.MaterialResp, error) {
	labUser := auth.GetCurrentUser(ctx)
	if labUser == nil {
		return nil, code.UnLogin
	}
	// TODO: implement material list query
	return nil, nil
}

func (m *materialImpl) BatchUpdateMaterial(ctx context.Context, _ *material.UpdateMaterialReq) error {
	labUser := auth.GetCurrentUser(ctx)
	if labUser == nil {
		return code.UnLogin
	}
	// TODO: implement batch update material
	return nil
}

func (m *materialImpl) BatchUpdateUniqueName(ctx context.Context, _ *material.UpdateMaterialReq) error {
	labUser := auth.GetCurrentUser(ctx)
	if labUser == nil {
		return code.UnLogin
	}
	// TODO: implement batch update unique name
	return nil
}

func (m *materialImpl) CreateEdge(ctx context.Context, _ *material.GraphEdge) error {
	labUser := auth.GetCurrentUser(ctx)
	if labUser == nil {
		return code.UnLogin
	}
	// TODO: implement create edge
	return nil
}

func (m *materialImpl) OnWSMsg(ctx context.Context, s *melody.Session, b []byte) error {
	wsMsg := &common.WsMsgType{}
	if err := json.Unmarshal(b, wsMsg); err != nil {
		logger.Errorf(ctx, "OnWSMsg unmarshal err: %+v", err)
		return code.UnmarshalWSDataErr
	}

	switch material.WSAction(wsMsg.Action) {
	case material.FetchGraph:
		return m.onFetchGraph(ctx, s, b)
	case material.CreateNode:
		return m.onCreateNode(ctx, s, b)
	case material.UpdateNode:
		return m.onUpdateNode(ctx, s, b)
	case material.BatchDeleteNode:
		return m.onBatchDeleteNode(ctx, s, b)
	case material.BatchCreateEdge:
		return m.onBatchCreateEdge(ctx, s, b)
	case material.BatchDeleteEdge:
		return m.onBatchDeleteEdge(ctx, s, b)
	case material.UpdateNodeData:
		return m.onUpdateNodeData(ctx, s, b)
	default:
		logger.Errorf(ctx, "unknown ws action: %s", wsMsg.Action)
		return code.UnknownWSActionErr
	}
}

func (m *materialImpl) OnWSConnect(ctx context.Context, _ *melody.Session) error {
	logger.Infof(ctx, "material ws connect")
	return nil
}

func (m *materialImpl) OnMaterialNotify(ctx context.Context, msg string) error {
	logger.Infof(ctx, "material notify: %s", msg)
	// Broadcast to all connected WS sessions for the lab
	return m.wsClient.Broadcast([]byte(msg))
}

func (m *materialImpl) DownloadMaterial(_ context.Context, _ *material.DownloadMaterial) (*material.GraphNodeReq, error) {
	// TODO: implement material download
	return &material.GraphNodeReq{}, nil
}

func (m *materialImpl) GetMaterialTemplate(_ context.Context, _ *material.TemplateReq) (*material.TemplateResp, error) {
	// TODO: implement material template query
	return &material.TemplateResp{}, nil
}

func (m *materialImpl) GetResourceNodeTemplate(_ context.Context, _ *material.AllTemplateReq) (*material.ResourceTemplates, error) {
	// TODO: implement resource node template query
	return &material.ResourceTemplates{}, nil
}

func (m *materialImpl) ResourceList(_ context.Context, _ *material.ResourceReq) (*material.ResourceResp, error) {
	// TODO: implement resource list
	return &material.ResourceResp{}, nil
}

func (m *materialImpl) DeviceAction(_ context.Context, _ *material.DeviceActionReq) (*material.DeviceActionResp, error) {
	// TODO: implement device action
	return &material.DeviceActionResp{}, nil
}

func (m *materialImpl) StartMachine(_ context.Context, _ *material.StartMachineReq) (*material.StartMachineRes, error) {
	// TODO: implement start machine
	return &material.StartMachineRes{}, nil
}

func (m *materialImpl) DelMachine(_ context.Context, _ *material.DelMachineReq) error {
	// TODO: implement delete machine
	return nil
}

func (m *materialImpl) StopMachine(_ context.Context, _ *material.StopMachineReq) error {
	// TODO: implement stop machine
	return nil
}

func (m *materialImpl) MachineStatus(_ context.Context, _ *material.MachineStatusReq) (*material.MachineStatusRes, error) {
	// TODO: implement machine status
	return &material.MachineStatusRes{}, nil
}

// WS handler helpers

func (m *materialImpl) onFetchGraph(ctx context.Context, s *melody.Session, _ []byte) error {
	logger.Infof(ctx, "onFetchGraph")
	return common.ReplyWSOk(s, string(material.FetchGraph), common.WsMsgType{}.MsgUUID)
}

func (m *materialImpl) onCreateNode(ctx context.Context, _ *melody.Session, _ []byte) error {
	logger.Infof(ctx, "onCreateNode")
	return nil
}

func (m *materialImpl) onUpdateNode(ctx context.Context, _ *melody.Session, _ []byte) error {
	logger.Infof(ctx, "onUpdateNode")
	return nil
}

func (m *materialImpl) onBatchDeleteNode(ctx context.Context, _ *melody.Session, _ []byte) error {
	logger.Infof(ctx, "onBatchDeleteNode")
	return nil
}

func (m *materialImpl) onBatchCreateEdge(ctx context.Context, _ *melody.Session, _ []byte) error {
	logger.Infof(ctx, "onBatchCreateEdge")
	return nil
}

func (m *materialImpl) onBatchDeleteEdge(ctx context.Context, _ *melody.Session, _ []byte) error {
	logger.Infof(ctx, "onBatchDeleteEdge")
	return nil
}

func (m *materialImpl) onUpdateNodeData(ctx context.Context, _ *melody.Session, _ []byte) error {
	logger.Infof(ctx, "onUpdateNodeData")
	return nil
}
