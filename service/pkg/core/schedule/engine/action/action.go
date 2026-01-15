package action

import (
	// 外部依赖
	"context"
	"encoding/json"
	"sync"
	"time"

	melody "github.com/olahol/melody"
	r "github.com/redis/go-redis/v9"

	// 内部引用
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	uuid "github.com/scienceol/opensdl/service/pkg/common/uuid"
	notify "github.com/scienceol/opensdl/service/pkg/core/notify"
	engine "github.com/scienceol/opensdl/service/pkg/core/schedule/engine"
	lab "github.com/scienceol/opensdl/service/pkg/core/schedule/lab"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	redis "github.com/scienceol/opensdl/service/pkg/middleware/redis"
	repo "github.com/scienceol/opensdl/service/pkg/repo"
	model "github.com/scienceol/opensdl/service/pkg/model"
	utils "github.com/scienceol/opensdl/service/pkg/utils"
)

type stepFunc func(ctx context.Context) error

type actionEngine struct {
	job     *engine.WorkflowInfo
	cancel  context.CancelFunc
	ctx     context.Context
	session *melody.Session
	data    *RunActionReq
	ret     *RunActionResp

	wg        sync.WaitGroup
	stepFuncs []stepFunc

	boardEvent notify.MsgCenter

	actionStatus sync.Map
	rClient      *r.Client
	sanbox       repo.Sandbox
}

func NewActionTask(ctx context.Context, param *engine.ActionParam) engine.Task {
	cancelCtx, cancel := context.WithCancel(ctx)

	d := &actionEngine{
		session: param.Session,
		cancel:  cancel,
		ctx:     cancelCtx,
		wg:      sync.WaitGroup{},
		rClient: redis.GetClient(),
		sanbox:  param.Sandbox,
		job:     param.WorkflowInfo,
	}
	d.stepFuncs = append(d.stepFuncs,
		d.loadData, // 加载运行数据
	)

	return d
}

func (d *actionEngine) loadData(ctx context.Context) error {
	paramKey := ActionKey(d.job.TaskUUID)
	paramRet := d.rClient.Get(ctx, paramKey)
	if paramRet.Err() != nil {
		logger.Errorf(ctx, "actionEngine loadData err: %+v", paramRet.Err())
		return paramRet.Err()
	}

	data := &RunActionReq{}
	err := json.Unmarshal([]byte(paramRet.Val()), data)
	if err != nil {
		logger.Errorf(ctx, "actionEngine loadData Unmarshal err: %+v", err)
		return code.ParamErr.WithErr(err)
	}

	if data.UUID.IsNil() ||
		data.LabUUID.IsNil() ||
		data.Action == "" ||
		data.ActionType == "" ||
		data.DeviceID == "" {
		logger.Errorf(ctx, "actionEngine loadData pararm err: %+v", err)
		return code.ParamErr
	}

	d.data = data
	return nil
}

// 运行入口
func (d *actionEngine) Run() error {
	var err error
	defer func() {
		d.setActionRet(d.ctx)
	}()

	for _, s := range d.stepFuncs {
		if err = s(d.ctx); err != nil {
			break
		}
	}
	err = d.runNode(d.ctx)
	return err
}

func (d *actionEngine) Stop() error {
	return nil
}

func (d *actionEngine) runNode(ctx context.Context) error {
	// 查询 action 是否可以执行
	err := d.queryAction(ctx)
	if err != nil {
		return err
	}

	err = d.sendAction(ctx)
	if err != nil {
		return err
	}

	key := engine.ActionKey{
		Type:       engine.JobCallbackStatus,
		TaskUUID:   d.job.TaskUUID,
		JobID:      d.job.TaskUUID,
		DeviceID:   d.data.DeviceID,
		ActionName: d.data.Action,
	}

	d.InitDeviceActionStatus(ctx, key, time.Now().Add(20*time.Second), false)
	err = d.callbackAction(ctx, key)

	return err
}

func (d *actionEngine) queryAction(ctx context.Context) error {
	key := engine.ActionKey{
		Type:     engine.QueryActionStatus,
		TaskUUID: d.job.TaskUUID,
		JobID:    d.job.TaskUUID,
		DeviceID: utils.SafeValue(func() string {
			return d.data.DeviceID
		}, ""),
		ActionName: d.data.Action,
	}
	d.InitDeviceActionStatus(ctx, key, time.Now().Add(time.Second*20), false)
	if err := d.sendQueryAction(ctx); err != nil {
		return err
	}

	for {
		select {
		case <-ctx.Done():
			return code.JobCanceled
		default:
		}

		time.Sleep(time.Millisecond * 500)
		value, exist := d.GetDeviceActionStatus(ctx, key)
		if !exist {
			return code.QueryJobStatusKeyNotExistErr
		}

		if value.Free {
			d.DelStatus(ctx, key)
			return nil
		}

		if value.Timestamp.Unix() < time.Now().Unix() {
			return code.JobTimeoutErr
		}
	}
}

func (d *actionEngine) sendQueryAction(_ context.Context) error {
	if d.session.IsClosed() {
		return code.EdgeConnectClosedErr
	}

	data := lab.EdgeData[engine.ActionKey]{
		EdgeMsg: lab.EdgeMsg{
			Action: lab.QueryActionStatus,
		},
		Data: engine.ActionKey{
			TaskUUID:   d.job.TaskUUID,
			JobID:      d.job.TaskUUID,
			DeviceID:   d.data.DeviceID,
			ActionName: d.data.Action,
		},
	}

	bData, _ := json.Marshal(data)
	return d.session.Write(bData)
}

func (d *actionEngine) sendAction(_ context.Context) error {
	if d.session.IsClosed() {
		return code.EdgeConnectClosedErr
	}

	data := lab.EdgeData[engine.SendActionData]{
		EdgeMsg: lab.EdgeMsg{
			Action: lab.JobStart,
		},
		Data: engine.SendActionData{
			DeviceID:   d.data.DeviceID,
			Action:     d.data.Action,
			ActionType: d.data.ActionType,
			ActionArgs: d.data.Param,
			JobID:      d.job.TaskUUID,
			TaskID:     d.job.TaskUUID,
			NodeID:     d.job.TaskUUID,
			ServerInfo: engine.ServerInfo{
				SendTimestamp: float64(time.Now().UnixNano()) / 1e9,
			},
		},
	}

	b, err := json.Marshal(data)
	if err != nil {
		return code.NodeDataMarshalErr.WithErr(err)
	}

	return d.session.Write(b)
}

func (d *actionEngine) callbackAction(ctx context.Context, key engine.ActionKey) error {
	for {
		select {
		case <-ctx.Done():
			return code.JobCanceled
		default:
		}

		time.Sleep(time.Millisecond * 500)
		value, exist := d.GetDeviceActionStatus(ctx, key)
		if !exist {
			return code.CallbackJobStatusKeyNotExistErr
		}

		if value.Free {
			d.DelStatus(ctx, key)
			break
		}

		if value.Timestamp.Unix() < time.Now().Unix() {
			return code.JobTimeoutErr
		}
	}

	// 查询任务状态是否回调成功
	if d.ret == nil {
		return code.JobTimeoutErr
	}

	logger.Infof(ctx, "schedule action job run finished: %d", d.job.TaskID)
	switch d.ret.Status {
	case string(model.WorkflowJobSuccess):
		return nil
	case string(model.WorkflowJobFailed):
		return code.JobRunFailErr
	default:
		return code.JobRunFailErr
	}
}

func (d *actionEngine) GetStatus(_ context.Context) error {
	return nil
}

func (d *actionEngine) OnJobUpdate(ctx context.Context, data *engine.JobData) error {
	if data.Status == "running" {
		return nil
	}

	d.SetDeviceActionStatus(ctx, engine.ActionKey{
		Type:       engine.JobCallbackStatus,
		TaskUUID:   data.TaskID,
		JobID:      data.JobID,
		DeviceID:   data.DeviceID,
		ActionName: data.ActionName,
	}, true, 0)

	d.ret = &RunActionResp{
		JobData: data,
	}

	return nil
}

func (d *actionEngine) setActionRet(ctx context.Context) {
	retKey := ActionRetKey(d.job.TaskUUID)
	value := &RunActionResp{}
	if d.ret == nil {
		if d.job != nil {
			value.Status = "fail"
			value.JobID = d.job.TaskUUID
			value.TaskID = d.job.TaskUUID
			value.Status = "fail"
		} else {
			value.Status = "fail"
		}
	} else {
		value = d.ret
	}

	b, _ := json.Marshal(value)
	ret := d.rClient.SetEx(ctx, retKey, b, 1*time.Hour)
	if ret.Err() != nil {
		logger.Errorf(ctx, "setActionRet err: %+v", ret.Err())
	}
}

func (d *actionEngine) GetDeviceActionStatus(ctx context.Context, key engine.ActionKey) (engine.ActionValue, bool) {
	valueI, ok := d.actionStatus.Load(key)
	if !ok {
		return engine.ActionValue{}, false
	}
	return valueI.(engine.ActionValue), true
}

func (d *actionEngine) SetDeviceActionStatus(ctx context.Context, key engine.ActionKey, free bool, needMore time.Duration) {
	valueI, ok := d.actionStatus.Load(key)
	if ok {
		value := valueI.(engine.ActionValue)
		value.Free = free
		value.Timestamp = value.Timestamp.Add(needMore)
		logger.Warnf(ctx, "SetDeviceActionStatus key: %+v, value: %+v, more: %d", key, value, needMore)
		d.actionStatus.Store(key, value)
	} else {
		logger.Warnf(ctx, "SetDeviceActionStatus not found key: %+v", key)
	}
}

func (d *actionEngine) InitDeviceActionStatus(ctx context.Context, key engine.ActionKey, start time.Time, free bool) {
	d.actionStatus.Store(key, engine.ActionValue{
		Timestamp: start,
		Free:      free,
	})
}

func (d *actionEngine) DelStatus(ctx context.Context, key engine.ActionKey) {
	d.actionStatus.Delete(key)
}

func (d *actionEngine) Type(ctx context.Context) engine.JobType {
	return engine.ActionJobType
}

func (d *actionEngine) ID(ctx context.Context) uuid.UUID {
	if d.job == nil {
		return uuid.NewNil()
	}

	return d.job.TaskUUID
}
