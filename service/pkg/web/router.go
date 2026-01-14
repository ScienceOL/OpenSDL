package web

import (
	// 外部依赖
	"context"
	"fmt"

	gin "github.com/gin-gonic/gin"
	cors "github.com/gin-contrib/cors"
	_ "github.com/scienceol/opensdl/service/docs" // 导入自动生成的 docs 包
	swaggerfiles "github.com/swaggo/files"
	ginSwagger "github.com/swaggo/gin-swagger"
	otelgin "go.opentelemetry.io/contrib/instrumentation/github.com/gin-gonic/gin/otelgin"

	// 内部引用
	config "github.com/scienceol/opensdl/service/internal/config"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	views "github.com/scienceol/opensdl/service/pkg/web/views"
	inner "github.com/scienceol/opensdl/service/pkg/web/views/inner"
	login "github.com/scienceol/opensdl/service/pkg/web/views/login"
	auth "github.com/scienceol/opensdl/service/pkg/middleware/auth"
	foo "github.com/scienceol/opensdl/service/pkg/web/views/foo"
	laboratory "github.com/scienceol/opensdl/service/pkg/web/views/laboratory"
	material "github.com/scienceol/opensdl/service/pkg/web/views/material"
	workflow "github.com/scienceol/opensdl/service/pkg/web/views/workflow"
	mcp "github.com/scienceol/opensdl/service/pkg/web/views/mcp"
	notebook "github.com/scienceol/opensdl/service/pkg/web/views/notebook"
	reagent "github.com/scienceol/opensdl/service/pkg/web/views/reagent"
	storage "github.com/scienceol/opensdl/service/pkg/web/views/storage"
)

func NewRouter(ctx context.Context, g *gin.Engine) {
	installMiddleware(g)
	InstallURL(ctx, g)
}

func installMiddleware(g *gin.Engine) {
	// TODO: trace 中间件
	g.ContextWithFallback = true
	server := config.Global().Server
	g.Use(cors.Default())
	g.Use(otelgin.Middleware(fmt.Sprintf("%s-%s",
		server.Platform,
		server.Service)))
	g.Use(logger.LogWithWriter())
}

func InstallURL(ctx context.Context, g *gin.Engine) {
	api := g.Group("/api")
	api.GET("/health", views.Health)
	api.GET("/swagger/*any", ginSwagger.WrapHandler(swaggerfiles.Handler))

	{
		innerConf := config.Global().Inner
		innerGroup := api.Group("inner", gin.BasicAuth(gin.Accounts{
			innerConf.BasicAuthUser: innerConf.BasicAuthPassword,
		}))
		h := inner.New()
		innerGroup.GET("/policy", h.GetUserCustomPolicy)
		innerGroup.GET("/policy/resource", h.GetPolicyResource)

	}

	// 登录模块
	{
		l := login.NewLogin()
		// 设置认证相关路由
		authGroup := api.Group("/auth")
		// 登录路由 - 使用改进版处理器
		authGroup.GET("/login", l.Login)
		// OAuth2 回调处理路由 - 使用改进版处理器
		authGroup.GET("/callback/casdoor", l.Callback)
		// 刷新令牌路由
		authGroup.POST("/refresh", l.Refresh)
	}

	{
		v1 := api.Group("/v1")
		// 设置测试路由
		fooGroup := v1.Group("/foo")
		// 设置一个需要认证的路由 - 使用 RequireAuth 中间件进行验证
		fooGroup.GET("/hello", auth.AuthWeb(), foo.HandleHelloWorld)
		wsRouter := v1.Group("/ws", auth.AuthWeb())

		// 环境相关
		{
			labRouter := v1.Group("/lab", auth.AuthWeb())

			{
				labHandle := laboratory.NewEnvironment()
				labRouter.POST("", labHandle.CreateLabEnv)                                 // 创建实验室
				labRouter.PATCH("", labHandle.UpdateLabEnv)                                // 更新实验室
				labRouter.DELETE("", labHandle.DelLabEnv)                                  // 更新实验室
				labRouter.GET("/list", labHandle.LabList)                                  // 获取当前用户的所有实验室
				labRouter.GET("/info/:uuid", labHandle.LabInfo)                            // 获取当前用户的所有实验室
				labRouter.POST("/resource", labHandle.CreateLabResource)                   // 从 edge 侧创建资源
				labRouter.GET("/member/:lab_uuid", labHandle.GetLabMember)                 // 根据实验室获取当前实验室成员
				labRouter.DELETE("/member/:lab_uuid/:member_uuid", labHandle.DelLabMember) // 删除实验室成员
				labRouter.POST("/invite/:lab_uuid", labHandle.CreateInvite)                // 创建邀请链接
				labRouter.GET("/invite/:uuid", labHandle.AcceptInvite)                     // 接受邀请链接
				labRouter.GET("/user/info", labHandle.UserInfo)                            // 获取用户信息
				labRouter.PATCH("/pin", labHandle.PinLab)                                  // 收藏实验室
				{
					// 权限相关
					labPolicyRouter := labRouter.Group("/policy")
					labPolicyRouter.GET("", labHandle.Policy)                    // 获取权限组
					labPolicyRouter.GET("/resource", labHandle.PolicyResource)   // 获取权限组
					labPolicyRouter.POST("/role", labHandle.CreateRole)          // 创建角色
					labPolicyRouter.GET("/role/list", labHandle.RoleList)        // 获取角色列表
					labPolicyRouter.DELETE("/role", labHandle.DelRole)           // 删除角色
					labPolicyRouter.GET("/role/perm", labHandle.RolePermList)    // 获取角色权限列表
					labPolicyRouter.POST("/role/perm", labHandle.ModifyRolePerm) // 创建角色权限
					labPolicyRouter.POST("/user/role", labHandle.ModifyUserRole) // 赋予或删除用户角色
				}
				{
					// 项目相关
					labProjectRouter := labRouter.Group("project")
					_ = labProjectRouter
				}
			}

			// FIXME: 后续优化
			{
				materialRouter := labRouter.Group("/material")
				materialHandle := material.NewMaterialHandle(ctx)
				materialRouter.POST("", materialHandle.CreateLabMaterial)                  //  创建物料 done
				materialRouter.GET("", materialHandle.QueryMaterial)                       // edge 侧查询物料资源
				materialRouter.PUT("", materialHandle.BatchUpdateMaterial)                 // edge 批量更新物料数据
				materialRouter.POST("/save", materialHandle.SaveMaterial)                  //  保存物料
				materialRouter.GET("/resource", materialHandle.ResourceList)               // 获取该实验室所有设备列表
				materialRouter.GET("/device/actions", materialHandle.Actions)              // 获取实验室所有动作
				materialRouter.POST("/edge", materialHandle.CreateMaterialEdge)            // 创建物料连线 done
				materialRouter.GET("/download/:lab_uuid", materialHandle.DownloadMaterial) // 下载物料dag done
				materialRouter.GET("/template/:template_uuid", materialHandle.Template)
				materialRouter.GET("/template", materialHandle.GetResourceNodeTemplate) // 获取resource类型的物料模板
				// 仿真相关
				materialRouter.POST("/machine", materialHandle.StartMahine)     // 仿真开机
				materialRouter.PUT("/machine", materialHandle.StopMachine)      // 仿真关机
				materialRouter.DELETE("/machine", materialHandle.DeleteMachine) // 删除仿真
				materialRouter.GET("/machine", materialHandle.MachineStatus)    // 查询仿真机器状态

				// 后续待优化, 单独拆出去。
				{
					// 实验室 edge 上报接口
					edgeRouter := v1.Group("/edge", auth.AuthWeb())
					materialRouter := edgeRouter.Group("/material")
					materialRouter.POST("", materialHandle.EdgeCreateMaterial)
					materialRouter.PUT("", materialHandle.EdgeUpsertMaterial) // 更新 & 创建
					materialRouter.POST("/edge", materialHandle.EdgeCreateEdge)
					materialRouter.POST("/query", materialHandle.QueryMaterialByUUID)
					materialRouter.GET("/download", materialHandle.EdgeDownloadMaterial)
					// materialRouter.PATCH("", materialHandle.EdgeCreateMaterial)
				}

				wsRouter.GET("/material/:lab_uuid", materialHandle.LabMaterial)
			}
			{
				workflowHandle := workflow.NewWorkflowHandle(ctx)
				workflowRouter := labRouter.Group("/workflow")
				workflowRouter.GET("/task/:uuid", workflowHandle.TaskList)              // 工作流 task 列表
				workflowRouter.GET("/task/download/:uuid", workflowHandle.DownloadTask) // 工作流任务下载
				workflowRouter.GET("/graph/:uuid", workflowHandle.GetWorkflowGraph)     // 获取工作流 DAG 图
				workflowRouter.GET("/devices", workflowHandle.DeviceList)               // 获取该实验室可执行的设备列表

				{
					// 工作流模板
					tpl := workflowRouter.Group("/template")
					tpl.GET("/detail/:uuid", workflowHandle.GetWorkflowDetail)           // 获取工作流模板详情
					tpl.PUT("/fork", workflowHandle.ForkTemplate)                        // fork 工作流 done
					tpl.GET("/tags", workflowHandle.WorkflowTemplateTags)                // 获取工作流 tags done
					tpl.GET("/tags/:lab_uuid", workflowHandle.WorkflowTemplateTagsByLab) // 按实验室获取工作流模板标签
					tpl.GET("/list", workflowHandle.WorkflowTemplateList)                // 获取工作流模板列表 done
				}
				{
					// 工作流节点模板
					nodeTpl := workflowRouter.Group("/node/template")
					nodeTpl.GET("/tags/:lab_uuid", workflowHandle.TemplateTags)     // 节点模板 tags done
					nodeTpl.GET("/list", workflowHandle.TemplateList)               // 模板列表 done
					nodeTpl.GET("/detail/:uuid", workflowHandle.NodeTemplateDetail) // 节点模板详情 done
					nodeTpl.PATCH("/modify", workflowHandle.NodeTemplateUpdate)
				}
				{
					// 我的工作流
					owner := workflowRouter.Group("owner")
					owner.PATCH("", workflowHandle.UpdateWorkflow)     // 更新工作流 done
					owner.POST("", workflowHandle.Create)              // 创建工作流 done
					owner.DELETE("/:uuid", workflowHandle.DelWrokflow) //  删除自己创建的工作流 done
					owner.GET("/list", workflowHandle.GetWorkflowList) // 获取工作流列表  done
					owner.GET("/export", workflowHandle.Export)        // 导出工作流
					owner.POST("/import", workflowHandle.Import)       // 导入工作流
					owner.PUT("/duplicate", workflowHandle.Duplicate)  // 复制工作流
				}
				{
					mcpRouter := labRouter.Group("/mcp")
					h := mcp.NewHandle()
					mcpRouter.POST("/run/action", h.RunAction) // 运行单个动作节点
					mcpRouter.GET("/task/:uuid", h.Task)       // 查询 task uuid 查询 task 状态
				}
				{
					// 实验记录本
					notebookRouter := labRouter.Group("/notebook")
					notebookHandle := notebook.NewNotebookHandle(ctx)
					notebookRouter.GET("/list", notebookHandle.QueryNotebook)
					notebookRouter.POST("", notebookHandle.CreateNotebook)
					notebookRouter.GET("/detail", notebookHandle.NotebookDetail) // 获取/notebook/详情（query）
					notebookRouter.DELETE("/:uuid", notebookHandle.DeleteNotebook)
					notebookRouter.GET("/schema", notebookHandle.NotebookSchema)
					notebookRouter.POST("/sample", notebookHandle.CreateSample)
					notebookRouter.GET("/sample", notebookHandle.GetNotebookBySample)
				}

				v1.PUT("/lab/run/workflow", workflowHandle.RunWorkflow)

				wsRouter.GET("/workflow/:uuid", workflowHandle.LabWorkflow)

				{
					reagentRouter := labRouter.Group("/reagent")
					reagentHandle := reagent.NewHandle()
					reagentRouter.POST("", reagentHandle.Insert)      // 插入
					reagentRouter.GET("/list", reagentHandle.Query)   // 查询
					reagentRouter.DELETE("", reagentHandle.Delete)    // 删除
					reagentRouter.PATCH("", reagentHandle.Update)     // 更新
					reagentRouter.GET("/cas", reagentHandle.QueryCAS) // 根据 CAS号 查询信息
				}
			}

			// 存储相关接口
			{
				storageRouter := labRouter.Group("/storage")
				storageHandle := storage.NewStorageHandle()
				storageRouter.GET("/token", storageHandle.CreateStoragePathToken)
			}
		}

	}
}
