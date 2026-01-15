package edge

import (
	// 外部依赖
	"context"
	"encoding/json"
	"reflect"
	"time"

	melody "github.com/olahol/melody"

	// 内部引用
	uuid "github.com/scienceol/opensdl/service/pkg/common/uuid"
	material "github.com/scienceol/opensdl/service/pkg/core/material"
	notify "github.com/scienceol/opensdl/service/pkg/core/notify"
	engine "github.com/scienceol/opensdl/service/pkg/core/schedule/engine"
	lab "github.com/scienceol/opensdl/service/pkg/core/schedule/lab"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	model "github.com/scienceol/opensdl/service/pkg/model"
	utils "github.com/scienceol/opensdl/service/pkg/utils"
)

// 处理 edge 侧消息
// edge 侧发送消息
func (e *EdgeImpl) OnEdgeMessge(ctx context.Context, s *melody.Session, b []byte) {
	logger.Infof(ctx, "schedule msg OnEdgeMessge job msg: %s", string(b))
	edgeType := &lab.EdgeMsg{}
	err := json.Unmarshal(b, edgeType)
	if err != nil {
		logger.Errorf(ctx, "OnEdgeMessge job msg Unmarshal err: %+v", err)
		return
	}

	switch edgeType.Action {
	case lab.JobStatus:
		e.OnJobStatus(ctx, s, b)
	case lab.DeviceStatus:
		e.OnDeviceStatus(ctx, s, b)
	case lab.Ping:
		e.OnPing(ctx, s, b)
	case lab.ReportActionState:
		e.onActionState(ctx, s, b)
	case lab.HostNodeReady:
		e.onEdgeReady(ctx, s, b)
	case lab.NormalExist:
		e.onNormalExit(ctx, s, b)
	default:
		logger.Errorf(ctx, "EdgeImpl.OnEdgeMessge unknow action: %s", edgeType.Action)
	}
}

func (e *EdgeImpl) OnJobStatus(ctx context.Context, s *melody.Session, b []byte) {
	res := lab.EdgeData[*engine.JobData]{}
	if err := json.Unmarshal(b, &res); err != nil {
		logger.Errorf(ctx, "onJobStatus err: %+v", err)
		return
	}

	e.onActionTask(ctx, &res)
	e.onJobTask(ctx, &res)
}

func (e *EdgeImpl) onActionTask(ctx context.Context, updateData *lab.EdgeData[*engine.JobData]) {
	if updateData == nil || updateData.Data == nil {
		return
	}

	if e.isTaskNil(ctx, e.actionTask) {
		return
	}

	if e.actionTask.ID(ctx) == updateData.Data.TaskID {
		e.actionTask.OnJobUpdate(ctx, updateData.Data)
	}
}

func (e *EdgeImpl) isTaskNil(_ context.Context, t engine.Task) bool {
	if t == nil {
		return true
	}

	v := reflect.ValueOf(t)
	if v.Kind() == reflect.Ptr && v.IsNil() {
		return true
	}

	return false
}

func (e *EdgeImpl) onJobTask(ctx context.Context, updateData *lab.EdgeData[*engine.JobData]) {
	if updateData == nil || updateData.Data == nil {
		return
	}

	if e.isTaskNil(ctx, e.jobTask) {
		return
	}

	if e.jobTask.ID(ctx) == updateData.Data.TaskID {
		e.jobTask.OnJobUpdate(ctx, updateData.Data)
	}
}

func (e *EdgeImpl) OnPing(ctx context.Context, s *melody.Session, b []byte) {
	req := lab.EdgeData[lab.ActionPong]{}
	if err := json.Unmarshal(b, &req); err != nil {
		logger.Errorf(ctx, "onActionState err: %+v", err)
		return
	}

	req.Data.ServerTimestamp = float64(time.Now().UnixMilli()) / 1000
	e.sendAction(ctx, s, &lab.EdgeData[any]{
		EdgeMsg: lab.EdgeMsg{
			Action: lab.Pong,
		},
		Data: req.Data,
	})
}

func (e *EdgeImpl) sendAction(ctx context.Context, s *melody.Session, data any) {
	bData, _ := json.Marshal(data)
	if err := s.Write(bData); err != nil {
		logger.Errorf(ctx, "EdgeImpl.sendAction err: %+v", err)
	}
}

func (e *EdgeImpl) OnDeviceStatus(ctx context.Context, s *melody.Session, b []byte) {
	res := lab.EdgeData[lab.DeviceData]{}
	if err := json.Unmarshal(b, &res); err != nil {
		logger.Errorf(ctx, "onJobStatus err: %+v", err)
		return
	}

	if res.Data.DeviceID == "" {
		logger.Errorf(ctx, "can not get device name: %s", string(b))
		return
	}

	valueI, ok := s.Get("lab_uuid")
	if !ok {
		logger.Warnf(ctx, "onDeviceStatus can not found uuid")
		return
	}
	labUUID, _ := valueI.(uuid.UUID)

	valueIDI, ok := s.Get("lab_id")
	if !ok {
		logger.Warnf(ctx, "onDeviceStatus can not found uuid")
		return
	}

	labID, _ := valueIDI.(int64)

	nodes, err := e.materialStore.UpdateMaterialNodeDataKey(ctx, labID,
		res.Data.DeviceID, res.Data.Data.PropertyName,
		res.Data.Data.Status)
	if err != nil {
		logger.Errorf(ctx, "onDeviceStatus update material data err: %+v", err)
		return
	}

	data := utils.FilterSlice(nodes, func(n *model.MaterialNode) (*material.UpdateMaterialData, bool) {
		return &material.UpdateMaterialData{
			UUID: n.UUID,
			Data: n.Data,
		}, true
	})

	d := material.UpdateMaterialDeviceNotify{
		Action: string(material.UpdateNodeData),
		Data:   data,
	}

	e.boardEvent.Broadcast(ctx, &notify.SendMsg{
		Channel:   notify.MaterialModify,
		LabUUID:   labUUID,
		UUID:      uuid.NewV4(),
		Data:      d,
		Timestamp: time.Now().Unix(),
	})
}

func (e *EdgeImpl) onActionState(ctx context.Context, _ *melody.Session, b []byte) {
	// 处理任务状态
	res := lab.EdgeData[lab.ActionStatus]{}
	if err := json.Unmarshal(b, &res); err != nil {
		logger.Errorf(ctx, "onActionState err: %+v", err)
		return
	}

	if res.Data.Type == "" ||
		res.Data.TaskUUID.IsNil() ||
		res.Data.JobID.IsNil() ||
		res.Data.DeviceID == "" ||
		res.Data.ActionName == "" {
		logger.Warnf(ctx, "onActionState param err: %+v", res)
		return
	}

	e.onAction(ctx, &res)
	e.onJob(ctx, &res)
}

func (e *EdgeImpl) onAction(ctx context.Context, data *lab.EdgeData[lab.ActionStatus]) {
	if data.Data.TaskUUID.IsNil() {
		return
	}

	if e.isTaskNil(ctx, e.actionTask) {
		return
	}

	if e.actionTask.ID(ctx) != data.Data.TaskUUID {
		return
	}

	e.actionTask.SetDeviceActionStatus(ctx, data.Data.ActionKey, data.Data.ActionValue.Free, data.Data.NeedMore*time.Second)
}

func (e *EdgeImpl) onJob(ctx context.Context, data *lab.EdgeData[lab.ActionStatus]) {
	if data.Data.TaskUUID.IsNil() {
		return
	}

	if e.isTaskNil(ctx, e.jobTask) {
		return
	}

	if e.jobTask.ID(ctx) != data.Data.TaskUUID {
		return
	}

	e.jobTask.SetDeviceActionStatus(ctx, data.Data.ActionKey, data.Data.ActionValue.Free, data.Data.NeedMore*time.Second)
}

func (e *EdgeImpl) onEdgeReady(ctx context.Context, _ *melody.Session, b []byte) {
	res := lab.EdgeData[lab.EdgeReady]{}
	if err := json.Unmarshal(b, &res); err != nil {
		logger.Errorf(ctx, "onActionState err: %+v", err)
		return
	}

	logger.Infof(ctx,
		"onEdgeReady lab id: %d, status: %s, timestamp: %f",
		e.labInfo.ID, res.Data.Status, res.Data.Timestamp)
	e.startTask(e.ctx)
	e.startControl(e.ctx)
	e.wait.Add(2)
}

func (e *EdgeImpl) onNormalExit(ctx context.Context, _ *melody.Session, _ []byte) {
	logger.Infof(ctx, "EdgeImpl.onNormalExit starting lab id: %d", e.labInfo.ID)
	e.Close(ctx)
}
