use std::{env, process::exit, fs, time, path::Path, sync::Arc};
use std::collections::HashMap;
use anyhow::{anyhow, Context, Result};
use std::io::{stdout, Stdout, Write};
use reqwest;
use serde::Deserialize;
use toml;
use url::Url;
use once_cell::sync::Lazy;
use tokio;
use crossterm::{cursor, QueueableCommand};

#[cfg(windows)]
const LINE_ENDING: &'static str = "\r\n";
#[cfg(not(windows))]
const LINE_ENDING: &'static str = "\n";

#[derive(Deserialize, Debug, Default)]
struct JenkinsExecPage {
    executable: Executable
}

#[derive(Deserialize, Debug, Default)]
struct Executable {
    url: String
}

#[derive(Deserialize)]
struct JenkinsResult {
    // null/SUCCESS/ABORTED/FAILURE
    result: Option<String>
}

#[derive(Deserialize, Debug)]
struct Config {
    jenkins: JenkinsConfig,
    file: FileConfig
}

#[derive(Deserialize, Debug)]
struct JenkinsConfig {
    build: Option<String>,
    poll_build_result_interval_second: Option<u64>,
    poll_build_result_counts: Option<u32>,
    instances: Vec<JenkinsInstanceConfig>,
}

#[derive(Deserialize, Debug, Default)]
struct JenkinsInstanceConfig {
    name: String,
    url: String,
    user: String,
    password: String,
    jobs: Option<HashMap<String, JenkinsJobConfig>>,
}

#[derive(Deserialize, Debug)]
struct JenkinsJobConfig {
    build: Option<String>,
    poll_build_result_interval_second: Option<u64>,
    poll_build_result_counts: Option<u32>,
    parameters: Option<HashMap<String, String>>
}


impl JenkinsJobConfig {
    fn get_build(&self) -> Result<&str> {
        match &self.build {
            Some(v) => Ok(v.as_str()),
            None => {
                match &CONFIG.jenkins.build {
                    Some(v) => Ok(v.as_str()),
                    None => Err(anyhow!("Missing job or global `build` configuration"))
                }
            }
        }
    }

    fn get_poll_build_result_interval_second<'a>(&self) -> Result<u64> {
        match &self.poll_build_result_interval_second {
            Some(v) => Ok(*v),
            None => {
                match &CONFIG.jenkins.poll_build_result_interval_second {
                    Some(v) => Ok(*v),
                    None => Err(anyhow!("Missing job or global `poll_build_result_interval_second` configuration"))
                }
            }
        }
    }

    fn get_poll_build_result_counts<'a>(&self) -> Result<u32> {
        match &self.poll_build_result_counts {
            Some(v) => Ok(*v),
            None => {
                match &CONFIG.jenkins.poll_build_result_counts {
                    Some(v) => Ok(*v),
                    None => Err(anyhow!("Missing job or global `poll_build_result_counts` configuration"))
                }
            }
        }
    }
}

impl Config {
    fn validate(&self) -> Result<()> {
        for instance in &self.jenkins.instances {
            instance.validate()?
        }
        Ok(())
    }

}

#[derive(Deserialize, Debug)]
struct FileConfig {
    path: String
}

#[derive(Debug)]
struct HttpClient {
    client: reqwest::Client,
    jenkins: &'static JenkinsInstanceConfig
}


static CONFIG: Lazy<Config> = Lazy::new(|| {
    let mut _args = env::args();
    let self_path = _args.next().unwrap();
    let c = _args.next();
    let config_path = match c {
        Some(v) => v,
        None => {
            let path = Path::new(&self_path);
            let parent = path.parent();
            if let None = parent {
                eprintln!("Failed to get parent directory of the program");
                exit(1);
            }
            let parent_absolute = fs::canonicalize(parent.unwrap()).unwrap();
            let p = parent_absolute.join("config.toml");
            let config_path = p.to_str().unwrap();
            config_path.to_string()
        }
    };
    let file_content = fs::read_to_string(&config_path);
    if let Err(e) = file_content {
        eprintln!("Failed to read the config file {:?}: {:?}", &config_path, e);
        exit(1);
    }
    let v = toml::from_str(&file_content.unwrap());
    if let Err(e) = v {
        eprintln!("Failed to parse the config file {:?}: {:?}", &config_path, e);
        exit(1)
    }
    let config: Config = v.unwrap();
    config
});

static JOB_FILE_CONTENT: Lazy<String> = Lazy::new(|| {
    let f = fs::read_to_string(&CONFIG.file.path);
    if let Err(e) = f {
        eprintln!("Failed to read {:?}: {:?}", &CONFIG.file.path, e);
        exit(1)
    }
    f.unwrap()
});

impl JenkinsInstanceConfig {
    fn validate(&self) -> Result<(), anyhow::Error> {
        let _ = Url::parse(&self.url).with_context(|| format!(
            "jenkins.instances.{}.url {}", &self.name, &self.url));
        Ok(())
    }
}

#[derive(Debug, Default, Copy, Clone)]
struct _JenkinsJobConfig {
    name: &'static str,
    instance_name: &'static str,
    build: &'static str,
    poll_build_result_interval_second: u64,
    poll_build_result_counts: u32,
    parameters: Option<&'static HashMap<String, String>>
}

impl _JenkinsJobConfig {
    fn set_value_from_initial(&mut self) -> Result<()> {
        self.build = &CONFIG.jenkins.build.as_ref().with_context(||
            format!("Missing job or global build configuration"))?;
        self.poll_build_result_counts = CONFIG.jenkins.poll_build_result_counts.with_context(||
            format!("Missing job or global poll_build_result_counts configuration"))?;
        self.poll_build_result_interval_second = CONFIG.jenkins.poll_build_result_interval_second.with_context(||
            format!("Missing job or global poll_build_result_interval_second configuration"))?;
        self.parameters = None;
        Ok(())
    }

    fn set_value_from_another(&mut self, obj: &'static JenkinsJobConfig) -> Result<()> {
        self.build = obj.get_build()?;
        self.poll_build_result_interval_second = obj.get_poll_build_result_interval_second()?;
        self.poll_build_result_counts = obj.get_poll_build_result_counts()?;
        match &obj.parameters {
            Some(map) => self.parameters = Some(&map),
            None => self.parameters = None
        }
        Ok(())
    }
}

impl HttpClient {
    fn new(jenkins_config: &'static JenkinsInstanceConfig) -> Result<Self> {
        let builder = reqwest::Client::builder();
        let client = builder.timeout(time::Duration::from_secs(3)).
            connect_timeout(time::Duration::from_secs(2)).
            tcp_keepalive(Some(time::Duration::from_secs(600).into())).
            build()?;
        Ok(HttpClient{client, jenkins: jenkins_config})
    }

    async fn job_build(&self, job_config: _JenkinsJobConfig) -> Result<String> {
        let u = Url::parse(&self.jenkins.url).unwrap();
        let tmp_url = String::from("/job/") + &job_config.name + "/" + job_config.build;
        let _u = u.join(&tmp_url)?;
        let url_str = _u.as_str();
        let response = match job_config.parameters {
            Some(v) => self.client.post(url_str).form(v).basic_auth(
                &self.jenkins.user, Some(&self.jenkins.password)).send().await.
            with_context(|| format!("Failed to get to {:?}", url_str))?,
            None => self.client.post(url_str).basic_auth(
                &self.jenkins.user,Some(&self.jenkins.password)).send().await.
                with_context(|| format!("Failed to get to {:?}", url_str))?
        };
        let headers = response.headers();
        let option = headers.get("Location").with_context(
            || format!("Failed to get Location in header that respond from posting to {:?}", url_str)
        )?;
        let location = option.to_str()?.to_string();
        Ok(location)
    }

    async fn get_job_status<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let mut i = 0;
        let t = loop {
            if i == 30 {
                return Err(anyhow!("Failed to get necessary field on {:?}", url))
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
            let response = self.client.get(url).basic_auth(
                &self.jenkins.user,Some(&self.jenkins.password)).send().await.with_context(||
                format!("Failed to get {:?}", url))?;
            let page = response.json::<T>().await.with_context(
                || format!("Failed to deserialize json on {:?}", url));
            if !page.is_err() {
                break page.unwrap()
            }
            i+=1;
        };
        Ok(t)
    }

    async fn get_job_result(&self, url: &str, job_config: _JenkinsJobConfig) -> Result<String> {
        let mut i = 0;
        loop {
            if i == job_config.poll_build_result_counts {
                return Err(anyhow!("Getting building result timeout on {:?}", url))
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(
                job_config.poll_build_result_interval_second)).await;
            let response = self.client.get(url).basic_auth(
                &self.jenkins.user,Some(&self.jenkins.password)).send().await.with_context(||
                format!("Failed to get {:?}", url))?;
            let page = response.json::<JenkinsResult>().await.with_context(
                || format!("Failed to deserialize json on {:?}", url))?;
            if let Some(result) = page.result {
                return Ok(result)
            }
            i+=1;
        };
    }
}


fn get_jenkins_clients() -> Result<HashMap<&'static str, HttpClient>> {
    let mut map: HashMap<&str, HttpClient> = HashMap::new();
    for instance in &CONFIG.jenkins.instances {
        let client = HttpClient::new(instance)?;
        map.insert(&instance.name, client);
    }
    Ok(map)
}

fn get_job_config(job: &'static str, jenkins_instance: &'static str) -> Result<_JenkinsJobConfig> {
    let mut jenkins_config = &CONFIG.jenkins.instances[0];
    for i in &CONFIG.jenkins.instances {
        if &i.name == jenkins_instance {
            jenkins_config = i;
        }
    }
    if &jenkins_config.name != jenkins_instance {
        return Err(anyhow!("No {} related jenkins configuration", jenkins_instance))
    }
    let mut job_config = _JenkinsJobConfig{
        instance_name: &jenkins_config.name,
        name: job,
        ..Default::default()};
    match &jenkins_config.jobs {
        Some(map) => {
            match map.get(job) {
                Some( value) => {
                    job_config.set_value_from_another(value)?;
                }
                None => {
                    job_config.set_value_from_initial().with_context(|| format!("{:?}", job))?;
                }
            }
        }
        None => {
            job_config.set_value_from_initial().with_context(|| format!("{:?}", job))?;
        }
    }
    Ok(job_config)
}

fn get_all_jobs() -> Result<Vec<_JenkinsJobConfig>> {
    let mut jenkins_instance: &str = &CONFIG.jenkins.instances[0].name;
    let mut jobs = Vec::new();
    for line in JOB_FILE_CONTENT.split(LINE_ENDING) {
        let trimmed_line = line.trim();
        if trimmed_line.len() == 0 {
            continue
        }
        if trimmed_line.starts_with('[') && trimmed_line.ends_with(']') {
            jenkins_instance = &trimmed_line[1..trimmed_line.len()-1];
            continue
        }
        let job_config = get_job_config(trimmed_line, jenkins_instance)?;

        jobs.push(job_config);
    }
    return Ok(jobs)
}

struct PrintData<'a> {
    v: Vec<String>,
    jobs: &'a Vec<_JenkinsJobConfig>,
    stdout: Stdout,
    counts: u16,
}

impl<'a> PrintData<'a> {
    fn new(jobs: &'a Vec<_JenkinsJobConfig>) -> Self {
        Self {
            v: vec![String::new(); jobs.len()],
            jobs,
            stdout: stdout(),
            counts: 0
        }
    }

    fn print(&mut self, idx: usize, result: String) {
        self.v[idx] = result;
        let mut content = String::new();
        // println!("{:?}", &self.v);
        if self.counts > 0 {
            let _ = self.stdout.queue(cursor::MoveUp(self.v.len() as u16));
            let _ = self.stdout.queue(cursor::MoveToColumn(1));
            let _ = self.stdout.flush();
        }
        for (idx, value) in self.v.iter().enumerate() {
            if value == "" {
                content += &format!("{} -> 发布中\n", &self.jobs[idx].name);
            } else {
                content += &format!("{} -> {}\n", &self.jobs[idx].name, value);
            }
        }
        print!("{}", content);
        self.counts += 1
    }
}


async fn exec() -> Result<String, anyhow::Error>{
    CONFIG.validate()?;
    let jenkins_clients = Arc::new(get_jenkins_clients()?);
    let jobs = get_all_jobs()?;
    let (tx, mut rx) = tokio::sync::mpsc::channel(jobs.len());
    let mut tasks = HashMap::with_capacity(jobs.len());
    for (idx, job) in jobs.iter().enumerate() {
        let i = job.clone();
        let tx = tx.clone();
        let jenkins_clients = jenkins_clients.clone();
        tasks.insert(i.name, tokio::spawn(async move {
            let client = jenkins_clients.get(i.instance_name).with_context(
                || format!("No jenkins instance named {} for job {}", i.instance_name, i.name))?;
            let location = client.job_build(i).await?;
            let jenkins_page = client.get_job_status::<JenkinsExecPage>(&(location + "api/json")).await?;
            let url = jenkins_page.executable.url + "api/json";
            client.get_job_status::<JenkinsResult>(&url).await?;
            let result = client.get_job_result(&url, i).await?;
            // println!("{:?} => {:?}", &i, &result);
            tx.send((idx, i.name)).await?;
            Ok::<String, anyhow::Error>(result)
        }));
    }
    drop(tx);

    let mut p = PrintData::new(&jobs);
    p.print(0, String::new());
    while let Some((idx, key)) = rx.recv().await {
        match tasks.remove(&key).unwrap().await {
            Ok(Ok(s)) => {p.print(idx, s)},
            Ok(Err(e)) => { p.print(idx, e.to_string()) },
            Err(e) => { p.print(idx, e.to_string()) }
        }
    }
    Ok(String::new())
}

#[tokio::main]
async fn main() {
    let v = exec().await;
    if let Err(e) = v {
        eprintln!("{:?}", e);
        exit(1)
    }
}
