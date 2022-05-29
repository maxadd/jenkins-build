通过提供 job 文件调用 Jenkins 进行发布。文件格式如下：

```ini
[jenkins_instance_name1]
job1
job2

[jenkins_instance_name2]
job2
job3
```

其中 `jenkins_instance_name` 可以省略，这个的名称对应的是配置文件中 jenkins 实例的名称，配置文件下面会提到。如果省略，会使用配置文件中第一个实例。因此最简单的 job 文件可以只有 job：

```
job1
job2
```

接下来就是配置文件，配置文件是 toml 格式，完整的配置文件如下：

```toml
# 这是全局配置，如果 job 配置中没有显式定义的话，使用全局配置
[jenkins]
# buildWithParameters 和 build 两种，一个是有参数一个是没有参数
build = "buildWithParameters"
# 多久遍历一次 job 的执行结果
poll_build_result_interval_second = 10
# 总共遍历多少次
poll_build_result_counts = 60

# jenkins 的实例列表
[[jenkins.instances]]
name = "dev"
url = "https://dev-jenkins.example.com"
user = "admin"
# 密码可以是 token 也可以是密码
password = "11287fa6fd10052b5513db2ec5ed14ad9z"

# 每个实例下面都可以有对应的 job 配置
[jenkins.instances.jobs.job1]
build = "buildWithParameters"
poll_build_result_interval_second = 10
poll_build_result_counts = 60

# job 如果有参数，可以写在这里
[jenkins.instances.jobs.job1.parameters]
app = "abc"
system = "efg"

# 第二个实例
[[jenkins.instances]]
name = "uat"
url = "https://uat-jenkins.example.com"
user = "admin"
password = "11287fa6fd10052b5513db2ec5ed14ad9z"

[jenkins.instances.jobs.job3]
build = "build"
poll_build_result_interval_second = 10
poll_build_result_counts = 60

# job 文件的路径
[file]
path = "/tmp/x"
```

编译方式：

在项目根目录下，执行 `cargo build --release`，不过依赖于 openssl-dev。生成的可执行文件在 target/release/jenkins-build。由于 rust 不同于 go，对 glibc 有依赖，无法做到一个包所有 Linux 发行版通吃，所以没有提供二进制文件。

执行方式：

```
./jenkins-build config.toml
```

如果将 config.toml 和二进制文件放在同一目录，那么直接执行就好，不需要任何参数。