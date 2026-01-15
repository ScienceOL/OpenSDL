package edge

import (
	// 外部依赖
	"context"
	"errors"
	"sync"
	"time"

	r "github.com/redis/go-redis/v9"

	// 内部引用
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	notify "github.com/scienceol/opensdl/service/pkg/core/notify"
	events "github.com/scienceol/opensdl/service/pkg/core/notify/events"
	engine "github.com/scienceol/opensdl/service/pkg/core/schedule/engine"
	lab "github.com/scienceol/opensdl/service/pkg/core/schedule/lab"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	redis "github.com/scienceol/opensdl/service/pkg/middleware/redis"
	repo "github.com/scienceol/opensdl/service/pkg/repo"
	mStore "github.com/scienceol/opensdl/service/pkg/repo/material"
	utils "github.com/scienceol/opensdl/service/pkg/utils"
)

/*
 该模块专注于设备 edge 连接管理、edge 消息转发、前端消息转发
*/

type EdgeImpl struct {
	sessionCtx    context.Context
	ctx           context.Context
	cancel        context.CancelFunc
	rClient       *r.Client // redis client
	labInfo       *lab.LabInfo
	jobTask       engine.Task // workflow or notebook task
	actionTask    engine.Task
	materialStore repo.MaterialRepo // 物料调度
	boardEvent    notify.MsgCenter  // 广播系统
	wait          sync.WaitGroup
	edgeSession   string // edge 侧连接的 session
}

func NewLab(ctx context.Context, labInfo *lab.LabInfo) (lab.Edge, error) {
	ctxCancel, cancel := context.WithCancel(context.Background())
	e := &EdgeImpl{
		sessionCtx:    ctx,
		ctx:           ctxCancel,
		cancel:        cancel,
		rClient:       redis.GetClient(),
		labInfo:       labInfo,
		materialStore: mStore.NewMaterialImpl(),
		boardEvent:    events.NewEvents(),
		wait:          sync.WaitGroup{},
		edgeSession:   labInfo.EdgeSession,
	}

	if err := e.startHeart(ctxCancel); err != nil {
		cancel()
		return nil, err
	}

	e.wait.Add(1)
	return e, nil
}

// 启动任务队列消费
func (e *EdgeImpl) startTask(ctx context.Context) {
	taskName := utils.LabTaskName(e.labInfo.UUID)
	utils.SafelyGo(func() {
		defer e.wait.Done()
		for {
			res, err := e.rClient.BRPop(ctx, 10*time.Second, taskName).Result()
			if err != nil && err == r.Nil {
				continue
			}

			if err != nil && errors.Is(err, context.Canceled) {
				logger.Infof(ctx, "EdgeImpl.startTask exit")
				return
			}

			if err != nil {
				logger.Warnf(ctx, "EdgeImpl.startTask err: %+v, name: %s", err, taskName)
				continue
			}

			if len(res) == 0 {
				logger.Warnf(ctx, "EdgeImpl.startTask err: %+v", err)
				continue
			}
			if err := utils.SafelyRun(func() {
				e.onJobMessage(ctx, res[1])
			}); err != nil {
				logger.Errorf(ctx, "EdgeImpl.onJobMessage err: %+v", err)
			}
		}
	}, func(err error) {
		logger.Errorf(ctx, "EdgeImpl.startTask SafelyGo err: %+v", err)
	})
}

// 启动控制命令队列消费
func (e *EdgeImpl) startControl(ctx context.Context) {
	controlName := utils.LabControlName(e.labInfo.UUID)
	utils.SafelyGo(func() {
		defer e.wait.Done()
		for {
			res, err := e.rClient.BRPop(ctx, 10*time.Second, controlName).Result()
			if err != nil && err == r.Nil {
				continue
			}

			if err != nil && errors.Is(err, context.Canceled) {
				logger.Infof(ctx, "EdgeImpl.startControl exit")
				return
			}

			if err != nil {
				logger.Warnf(ctx, "EdgeImpl.startControl err: %+v, name: %s", err, controlName)
				continue
			}

			if len(res) == 0 {
				logger.Warnf(ctx, "EdgeImpl.startControl err: %+v", err)
				continue
			}
			if err := utils.SafelyRun(func() {
				e.onControlMessage(ctx, res[1])
			}); err != nil {
				logger.Errorf(ctx, "EdgeImpl.onJobMessage err: %+v", err)
			}
		}
	}, func(err error) {
		logger.Errorf(ctx, "EdgeImpl.startControl SafelyGo err: %+v", err)
	})
}

// 启动活跃状态保证
func (e *EdgeImpl) startHeart(ctx context.Context) error {
	heartName := utils.LabHeartName(e.labInfo.UUID)
	heatTicker := time.Tick(utils.LabHeartTime)
	_, err := e.rClient.SetEx(ctx, heartName, e.edgeSession, utils.LabHeartTime+time.Second).Result()
	if err != nil {
		logger.Errorf(ctx, "EdgeImpl.startHeart set heart err: %+v", err)
		return code.SetLabHeartErr
	}
	utils.SafelyGo(func() {
		defer func() {
			e.rClient.Del(context.Background(), heartName)
			e.wait.Done()
		}()
		for {
			select {
			case <-ctx.Done():
				logger.Infof(ctx, "EdgeImpl.startHeart exit")
				return
			case <-heatTicker:
				_, err := e.rClient.SetEx(ctx, heartName, e.edgeSession, utils.LabHeartTime+time.Second).Result()
				if err != nil {
					logger.Errorf(ctx, "EdgeImpl.startHeart set heart err: %+v", err)
				}
			}
		}
	}, func(err error) {
		logger.Errorf(ctx, "EdgeImpl.startControl SafelyGo err: %+v", err)
	})

	return nil
}

// 处理关闭逻辑
func (e *EdgeImpl) Close(ctx context.Context) {
	if e.cancel != nil {
		e.cancel()
	}

	e.wait.Wait()
	logger.Infof(ctx, "EdgeImpl.Close exit lab id: %d", e.labInfo.ID)
}

func (e *EdgeImpl) OnPongMessage(ctx context.Context) {
}
